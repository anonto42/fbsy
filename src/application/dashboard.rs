//! Live full-screen service dashboard (`fbsy dashboard`).
//!
//! A ratatui front-end over the bridge service's registry/process core.
//! It auto-refreshes, shows a live log pane, and offers two ways to drive
//! the bridge service: single-key shortcuts and a `:command` bar.

use std::{
    io::{self, IsTerminal, Write},
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result};
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::{
    application::{service, service::ServiceStatus, sync_once},
    services::ServiceKind,
    support::paths,
};

const TICK: Duration = Duration::from_millis(250);
const LOG_TAIL: usize = 400;

/// Normal navigation vs typing a `:command`.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Mode {
    Normal,
    Command,
}

/// Which full-screen page the dashboard is showing.
///
/// The dashboard is a small page-based app: it launches on `Home` (logo +
/// prompt, centered), running any command redirects to `Output` (live
/// execution logs), and `?` opens `Help`. `Esc` always navigates back home.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Page {
    Home,
    Output,
    Help,
}

/// Captured output of a passthrough CLI command, shown in a scrollable overlay.
struct OutputView {
    title: String,
    lines: Vec<String>,
    scroll: usize,
}

/// Work that must run attached to the real terminal (outside the TUI).
enum AttachedTask {
    /// Re-run this same `fbsy` binary with the given args (install/update/…).
    Cli(Vec<String>),
    /// Interactive setup wizard (configure HRMS + devices).
    Setup,
    /// One-shot LAN scan for biometric devices.
    Scan,
}

struct App {
    page: Page,
    log_scroll: usize,
    help_scroll: usize,
    /// Some(view) while a captured CLI command's output overlay is open.
    output: Option<OutputView>,
    /// Set when a `:command` must run attached to the real terminal;
    /// performed by the event loop, which owns the terminal so it can
    /// suspend and resume the TUI.
    pending_attach: Option<AttachedTask>,
    mode: Mode,
    input: String,
    status: String,
    quit: bool,
}

impl App {
    fn new() -> Self {
        Self {
            page: Page::Home,
            log_scroll: 0,
            help_scroll: 0,
            output: None,
            pending_attach: None,
            mode: Mode::Normal,
            input: String::new(),
            status: "press ? for help · : for commands".to_string(),
            quit: false,
        }
    }

    /// Navigate to a page, resetting that page's scroll state.
    fn goto(&mut self, page: Page) {
        self.page = page;
        match page {
            Page::Output => self.log_scroll = 0,
            Page::Help => self.help_scroll = 0,
            Page::Home => {}
        }
    }
}

/// Run the dashboard. Requires an interactive terminal.
pub fn run() -> Result<()> {
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        println!("fbsy dashboard needs an interactive terminal.");
        return Ok(());
    }

    // ratatui::init() enters the alternate screen, enables raw mode, and installs
    // a panic hook that restores the terminal — so a crash never leaves it broken.
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal) -> Result<()> {
    let mut app = App::new();

    while !app.quit {
        let bridge = bridge_status(service::snapshot());
        let sync_line = last_sync_summary();
        terminal.draw(|frame| draw(frame, &app, &bridge, &sync_line))?;

        if !event::poll(TICK)? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match app.mode {
            Mode::Command => handle_command_key(&mut app, key.code),
            Mode::Normal => handle_normal_key(&mut app, key.code),
        }

        // An interactive command must own the real terminal: drop the TUI, run it
        // attached, then re-enter the alternate screen.
        if let Some(task) = app.pending_attach.take() {
            run_attached(terminal, task, &mut app.status);
        }
    }
    Ok(())
}

// ── Key handling ──────────────────────────────────────────────────────────────

fn handle_normal_key(app: &mut App, code: KeyCode) {
    // While a command-output overlay is open, keys scroll or close it.
    if let Some(view) = app.output.as_mut() {
        match code {
            KeyCode::Esc | KeyCode::Char('q') => app.output = None,
            KeyCode::Up | KeyCode::Char('k') => view.scroll = view.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => view.scroll = view.scroll.saturating_add(1),
            KeyCode::PageUp => view.scroll = view.scroll.saturating_sub(10),
            KeyCode::PageDown => view.scroll = view.scroll.saturating_add(10),
            _ => {}
        }
        return;
    }

    match app.page {
        Page::Home => handle_home_key(app, code),
        Page::Output => handle_output_key(app, code),
        Page::Help => handle_help_key(app, code),
    }
}

/// Home page: bridge action shortcuts, or start typing a command.
fn handle_home_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
        KeyCode::Char('?') => app.goto(Page::Help),
        KeyCode::Tab | KeyCode::BackTab => app.goto(Page::Output),
        KeyCode::Char(':') => {
            app.mode = Mode::Command;
            app.input.clear();
        }
        KeyCode::Char('s') => run_action(app, Action::Start(ServiceKind::AtBridge)),
        KeyCode::Char('x') => run_action(app, Action::Stop("bridge".to_string())),
        KeyCode::Char('r') => run_action(
            app,
            Action::Restart("bridge".to_string(), ServiceKind::AtBridge),
        ),
        KeyCode::Char('y') => run_action(app, Action::Sync(None)),
        KeyCode::Char(c) => {
            app.mode = Mode::Command;
            app.input.clear();
            app.input.push(c);
        }
        _ => {}
    }
}

/// Output page: scroll the live execution logs; Esc/Tab go back home.
fn handle_output_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => app.goto(Page::Home),
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Char('?') => app.goto(Page::Help),
        // Bridge shortcuts still work here; output refreshes in place.
        KeyCode::Char('s') => run_action(app, Action::Start(ServiceKind::AtBridge)),
        KeyCode::Char('x') => run_action(app, Action::Stop("bridge".to_string())),
        KeyCode::Char('r') => run_action(
            app,
            Action::Restart("bridge".to_string(), ServiceKind::AtBridge),
        ),
        KeyCode::Char('y') => run_action(app, Action::Sync(None)),
        KeyCode::Up | KeyCode::Char('k') => app.log_scroll = app.log_scroll.saturating_add(1),
        KeyCode::Down | KeyCode::Char('j') => app.log_scroll = app.log_scroll.saturating_sub(1),
        KeyCode::PageUp => app.log_scroll = app.log_scroll.saturating_add(10),
        KeyCode::PageDown => app.log_scroll = app.log_scroll.saturating_sub(10),
        KeyCode::Home => app.log_scroll = usize::MAX, // oldest
        KeyCode::End => app.log_scroll = 0,           // newest
        _ => {}
    }
}

/// Help page: scroll; Esc/q/? go back home.
fn handle_help_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Esc => app.goto(Page::Home),
        KeyCode::Up | KeyCode::Char('k') => app.help_scroll = app.help_scroll.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => app.help_scroll = app.help_scroll.saturating_add(1),
        KeyCode::PageUp => app.help_scroll = app.help_scroll.saturating_sub(10),
        KeyCode::PageDown => app.help_scroll = app.help_scroll.saturating_add(10),
        KeyCode::Home => app.help_scroll = 0,
        KeyCode::End => app.help_scroll = usize::MAX,
        _ => {}
    }
}

fn handle_command_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
            app.input.clear();
        }
        KeyCode::Enter => {
            let line = app.input.trim().to_string();
            app.mode = Mode::Normal;
            app.input.clear();
            if !line.is_empty() {
                execute_command(app, &line);
            }
        }
        KeyCode::Backspace => {
            app.input.pop();
        }
        KeyCode::Char(c) => app.input.push(c),
        _ => {}
    }
}

// ── Actions ───────────────────────────────────────────────────────────────────

enum Action {
    Start(ServiceKind),
    Stop(String),
    Restart(String, ServiceKind),
    Sync(Option<String>),
}

fn run_action(app: &mut App, action: Action) {
    app.goto(Page::Output);
    app.status = match action {
        Action::Start(kind) => match service::default_start(kind) {
            Ok(pid) => format!("started {} (pid {pid})", kind.name()),
            Err(e) => format!("start {}: {e}", kind.name()),
        },
        Action::Stop(name) => match service::stop_instance(&name) {
            Ok(true) => format!("stopped {name}"),
            Ok(false) => format!("{name} was not running"),
            Err(e) => format!("stop {name}: {e}"),
        },
        Action::Restart(name, kind) => match service::restart_instance(&name) {
            Ok(pid) => format!("restarted {name} (pid {pid})"),
            Err(_) if name == kind.name() => match service::default_start(kind) {
                Ok(pid) => format!("started {} (pid {pid})", kind.name()),
                Err(e) => format!("restart {}: {e}", kind.name()),
            },
            Err(e) => {
                format!("restart {name}: {e}")
            }
        },
        Action::Sync(device) => match sync_once::run_summary(None, device) {
            Ok(summary) => summary,
            Err(e) => format!("sync failed: {e}"),
        },
    };
}

/// Parse and run a `:command` line.
///
/// Supports the bridge control aliases (`start`, `stop`, `restart`, `sync`,
/// `logs`) plus passthrough to `install`/`uninstall`/`update` on this same
/// `fbsy` binary. Anything else is rejected as unrecognized.
fn execute_command(app: &mut App, line: &str) {
    let mut args = match split_command_line(line) {
        Ok(args) => args,
        Err(err) => {
            app.status = err;
            return;
        }
    };
    if args.first().is_some_and(|arg| arg == "fbsy") {
        args.remove(0);
    }
    if args.is_empty() {
        app.status = "type a command, for example: start".to_string();
        return;
    }

    match args[0].as_str() {
        "help" | "?" => {
            app.goto(Page::Help);
            return;
        }
        "quit" | "q" | "exit" => {
            app.quit = true;
            return;
        }
        "start" | "s" => {
            run_action(app, Action::Start(ServiceKind::AtBridge));
            return;
        }
        "stop" | "x" => {
            run_action(app, Action::Stop("bridge".to_string()));
            return;
        }
        "restart" | "r" => {
            run_action(
                app,
                Action::Restart("bridge".to_string(), ServiceKind::AtBridge),
            );
            return;
        }
        "sync" | "y" => {
            run_action(app, Action::Sync(None));
            return;
        }
        "l" | "logs" => {
            app.goto(Page::Output);
            app.status = "execution output — Esc goes back home".to_string();
            return;
        }
        "home" | "h" => {
            app.goto(Page::Home);
            app.status = "home".to_string();
            return;
        }
        "setup" | "config" => {
            app.pending_attach = Some(AttachedTask::Setup);
            app.status = "leaving dashboard for the setup wizard".to_string();
            return;
        }
        "scan" => {
            app.pending_attach = Some(AttachedTask::Scan);
            app.status = "leaving dashboard to scan the local network".to_string();
            return;
        }
        "install" | "uninstall" | "update" => {
            // Valid pass-through commands
        }
        other => {
            app.status = format!("error: unrecognized command '{other}'");
            return;
        }
    }

    if should_run_attached(&args) {
        app.pending_attach = Some(AttachedTask::Cli(args));
        app.status = "leaving dashboard temporarily for an interactive command".to_string();
    } else {
        app.output = Some(run_captured(&args));
        app.status = "command output opened; Esc closes it".to_string();
    }
}

fn should_run_attached(args: &[String]) -> bool {
    let has_flag = |short: &str, long: &str| args.iter().any(|arg| arg == short || arg == long);
    match args {
        [cmd, ..] if cmd == "install" || cmd == "uninstall" => true,
        [cmd, rest @ ..] if cmd == "update" => {
            !(rest.iter().any(|arg| arg == "--check")
                || has_flag("-y", "--yes")
                || rest.iter().any(|arg| arg == "--auto"))
        }
        _ => false,
    }
}

fn run_captured(args: &[String]) -> OutputView {
    let title = format!("fbsy {}", display_args(args));
    let output = std::env::current_exe()
        .context("locate current executable")
        .and_then(|exe| {
            Command::new(exe)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .context("run command")
        });

    let mut lines = Vec::new();
    lines.push(format!("$ {title}"));
    lines.push(String::new());

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            lines.extend(stdout.lines().map(str::to_string));
            lines.extend(stderr.lines().map(str::to_string));
            if lines.len() == 2 {
                lines.push("(no output)".to_string());
            }
            if !output.status.success() {
                lines.push(String::new());
                lines.push(format!(
                    "exit status: {}",
                    output.status.code().map_or_else(
                        || "terminated by signal".to_string(),
                        |code| code.to_string()
                    )
                ));
            }
        }
        Err(err) => lines.push(format!("failed to run command: {err}")),
    }

    OutputView {
        title,
        lines,
        scroll: 0,
    }
}

fn run_attached(
    terminal: &mut ratatui::DefaultTerminal,
    task: AttachedTask,
    status_text: &mut String,
) {
    ratatui::restore();
    println!();

    match task {
        AttachedTask::Cli(args) => {
            println!("$ fbsy {}", display_args(&args));
            println!();
            let status = std::env::current_exe()
                .context("locate current executable")
                .and_then(|exe| {
                    Command::new(exe)
                        .args(&args)
                        .stdin(Stdio::inherit())
                        .stdout(Stdio::inherit())
                        .stderr(Stdio::inherit())
                        .status()
                        .context("run attached command")
                });
            match status {
                Ok(status) if status.success() => {
                    *status_text = format!("fbsy {} completed", display_args(&args));
                }
                Ok(status) => {
                    *status_text = format!("fbsy {} exited with {status}", display_args(&args));
                }
                Err(err) => {
                    *status_text = format!("could not run fbsy {}: {err}", display_args(&args));
                }
            }
        }
        AttachedTask::Setup => {
            *status_text = match crate::application::setup::run() {
                Ok(()) => "setup finished — restart the bridge to apply changes".to_string(),
                Err(err) => format!("setup failed: {err}"),
            };
        }
        AttachedTask::Scan => {
            let opts = crate::application::scanner::ScanOptions::default();
            *status_text = match crate::application::scanner::run_scan(opts) {
                Ok(()) => "scan finished".to_string(),
                Err(err) => format!("scan failed: {err}"),
            };
        }
    }

    println!();
    print!("Press Enter to return to the dashboard...");
    let _ = io::stdout().flush();
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);
    *terminal = ratatui::init();
}

fn split_command_line(line: &str) -> std::result::Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;

    for ch in line.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '\'' | '"' if quote == Some(ch) => quote = None,
            '\'' | '"' if quote.is_none() => quote = Some(ch),
            c if c.is_whitespace() && quote.is_none() => {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }

    if escaped {
        current.push('\\');
    }
    if let Some(quote) = quote {
        return Err(format!("unclosed {quote} quote"));
    }
    if !current.is_empty() {
        words.push(current);
    }
    Ok(words)
}

fn display_args(args: &[String]) -> String {
    args.iter()
        .map(|arg| {
            if arg.chars().any(char::is_whitespace) {
                format!("{arg:?}")
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn draw(frame: &mut Frame, app: &App, bridge: &ServiceStatus, sync_line: &str) {
    let area = frame.area();
    if area.width < 60 || area.height < 12 {
        let msg = format!(
            "Terminal too small ({}x{}). Minimum is 60x12. Please resize.",
            area.width, area.height
        );
        let para = Paragraph::new(msg)
            .style(Style::default().fg(Color::Red).bold())
            .wrap(Wrap { trim: true })
            .alignment(ratatui::layout::Alignment::Center);
        let centered = centered_rect(60, 3, area);
        frame.render_widget(Clear, centered);
        frame.render_widget(para, centered);
        return;
    }

    // Centering horizontal split (Codex Page width: 75% of screen, capped at 96 columns)
    let page_width = (area.width * 75 / 100).clamp(60, 96);
    let horizontal_padding = area.width.saturating_sub(page_width) / 2;
    let page_areas = Layout::horizontal([
        Constraint::Length(horizontal_padding),
        Constraint::Length(page_width),
        Constraint::Length(horizontal_padding),
    ])
    .split(area);

    let active_area = page_areas[1];

    match app.page {
        Page::Home => draw_home_page(frame, active_area, app, bridge, sync_line),
        Page::Output => draw_output_page(frame, active_area, app, bridge, sync_line),
        Page::Help => draw_help(frame, frame.area(), app),
    }

    // A captured passthrough command's output floats above everything else.
    if let Some(view) = &app.output {
        draw_output(frame, frame.area(), view);
    }
}

/// Home page: vertically centered logo, prompt card, and guide tags.
fn draw_home_page(
    frame: &mut Frame,
    active_area: Rect,
    app: &App,
    bridge: &ServiceStatus,
    sync_line: &str,
) {
    let content_height = 6 + 1 + 5 + 1; // Logo(6) + Spacing(1) + Input(5) + Guides(1)
    let top_padding = active_area
        .height
        .saturating_sub(content_height + 1) // +1 for footer line
        / 2;

    let splits = Layout::vertical([
        Constraint::Length(top_padding),
        Constraint::Length(6), // logo
        Constraint::Length(1), // spacing
        Constraint::Length(5), // prompt input box
        Constraint::Length(1), // guides
        Constraint::Min(1),    // bottom padding
    ])
    .split(active_area);

    let footer_area = Rect {
        x: active_area.x,
        y: active_area.y + active_area.height.saturating_sub(1),
        width: active_area.width,
        height: 1,
    };

    draw_logo(frame, splits[1]);
    draw_status(frame, splits[3], app, bridge, sync_line);
    draw_guides(frame, splits[4], app);
    draw_footer(frame, footer_area);
}

/// Output page: status header, full-height execution logs, guides, footer.
fn draw_output_page(
    frame: &mut Frame,
    active_area: Rect,
    app: &App,
    bridge: &ServiceStatus,
    sync_line: &str,
) {
    let splits = Layout::vertical([
        Constraint::Length(5), // prompt/status card stays on top
        Constraint::Length(1), // guides row
        Constraint::Min(8),    // execution logs take the rest
        Constraint::Length(1), // footer
    ])
    .split(active_area);

    draw_status(frame, splits[0], app, bridge, sync_line);
    draw_guides(frame, splits[1], app);
    draw_logs(frame, splits[2], app);
    draw_footer(frame, splits[3]);
}

/// Full help: dashboard shortcuts, aliases, and the CLI passthrough model.
fn draw_help(frame: &mut Frame, full: Rect, app: &App) {
    let width = full.width.saturating_sub(4).min(90);
    let height = full.height.saturating_sub(4).min(35);
    let area = centered_rect(width, height, full);

    let dim = Style::default().dim();
    let cyan = Style::default().fg(Color::Cyan).bold();
    let yellow = Style::default().fg(Color::Yellow).bold();

    let row = |k: &str, d: &'static str| {
        Line::from(vec![Span::styled(format!("  {k:<26}"), cyan), d.into()])
    };
    let separator = || {
        Line::from(vec![Span::styled(
            "  ────────────────────────────────────────────────────────────────────────",
            dim,
        )])
    };

    let lines = vec![
        Line::from(vec![Span::styled(" Pages", yellow)]),
        row("Home", "logo + prompt; where you launch commands from"),
        row("Output", "live execution logs; opens whenever a command runs"),
        row("Help", "this page"),
        row("Tab", "home ⇄ output page"),
        row("Esc", "go back home (from home: quit)"),
        separator(),
        Line::from(vec![Span::styled(" Single-key shortcuts", yellow)]),
        row("s", "start the bridge service (opens output page)"),
        row("x", "stop the bridge service"),
        row("r", "restart the bridge service"),
        row("y", "run a one-off sync now"),
        row("↑/↓ or j/k", "output page: scroll logs"),
        row("PgUp / PgDn", "scroll logs older / newer"),
        row("Home / End", "jump to oldest / newest log lines"),
        row("? / q", "this help / quit"),
        separator(),
        Line::from(vec![Span::styled(" Command bar", yellow)]),
        Line::from(vec![
            "  Type a command, or press ".into(),
            Span::styled(":", cyan),
            " first.".into(),
        ]),
        row("start / s", "start the bridge service"),
        row("stop / x", "stop the bridge service"),
        row("restart / r", "restart the bridge service"),
        row("sync / y", "run a one-off sync now"),
        row("setup", "configure HRMS connection and devices (wizard)"),
        row("scan", "discover biometric devices on the local network"),
        row("logs / l", "open the output page"),
        row("home / h", "go back to the home page"),
        row("help / ?", "open this help page"),
        row("install", "run `fbsy install` (attached)"),
        row("uninstall", "run `fbsy uninstall` (attached)"),
        row("update", "run `fbsy update` (attached, unless --check/--yes/--auto)"),
        row("quit / q", "exit the dashboard"),
    ];

    let visible = area.height.saturating_sub(2) as usize;
    let max_scroll = lines.len().saturating_sub(visible);
    let scroll = app.help_scroll.min(max_scroll).min(u16::MAX as usize) as u16;

    let title = if max_scroll > 0 {
        format!(" help ({}/{max_scroll}) — ↑/↓ scroll · Esc home ", scroll)
    } else {
        " help — Esc home ".to_string()
    };

    frame.render_widget(Clear, area);
    let para = Paragraph::new(lines)
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(para, area);
}

fn draw_output(frame: &mut Frame, full: Rect, view: &OutputView) {
    let area = centered_rect(92, full.height.saturating_sub(4).max(8), full);
    let visible = area.height.saturating_sub(2) as usize;
    let max_scroll = view.lines.len().saturating_sub(visible);
    let scroll = view.scroll.min(max_scroll).min(u16::MAX as usize) as u16;
    let lines = view
        .lines
        .iter()
        .map(|line| Line::from(format!(" {line}"))) // padded
        .collect::<Vec<_>>();

    frame.render_widget(Clear, area);
    let title = format!(
        " {} [{scroll}/{max_scroll}] · ↑/↓ scroll · Esc close ",
        view.title
    );
    let para = Paragraph::new(lines)
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false })
        .block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(para, area);
}

/// A centered rectangle `width`×`height` cells (clamped to the frame).
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    Rect {
        x,
        y,
        width: w,
        height: h,
    }
}

fn draw_logo(frame: &mut Frame, area: Rect) {
    let logo = [
        "███████╗██████╗ ███████╗██╗   ██╗",
        "██╔════╝██╔══██╗██╔════╝╚██╗ ██╔╝",
        "█████╗  ██████╔╝███████╗ ╚████╔╝ ",
        "██╔══╝  ██╔══██╗╚════██║  ╚██╔╝  ",
        "██║     ██████╔╝███████║   ██║   ",
        "╚═╝     ╚═════╝ ╚══════╝   ╚═╝   ",
    ];
    let green = Color::Rgb(16, 163, 127);
    let mut lines = Vec::new();
    for line in &logo {
        lines.push(
            Line::from(vec![Span::styled(
                line.to_string(),
                Style::default().fg(green).bold(),
            )])
            .alignment(ratatui::layout::Alignment::Center),
        );
    }
    let para = Paragraph::new(lines);
    frame.render_widget(para, area);
}

fn draw_guides(frame: &mut Frame, area: Rect, app: &App) {
    let dim = Color::Rgb(100, 100, 100);
    let cyan = Color::Rgb(16, 163, 127);
    let spans = if app.page == Page::Output {
        vec![
            Span::styled("Esc ", Style::default().fg(cyan).bold()),
            Span::styled("home  ", Style::default().fg(dim)),
            Span::styled("↑/↓ ", Style::default().fg(cyan).bold()),
            Span::styled("scroll  ", Style::default().fg(dim)),
            Span::styled("? ", Style::default().fg(cyan).bold()),
            Span::styled("help  ", Style::default().fg(dim)),
            Span::styled("q ", Style::default().fg(cyan).bold()),
            Span::styled("quit", Style::default().fg(dim)),
        ]
    } else {
        vec![
            Span::styled("tab ", Style::default().fg(cyan).bold()),
            Span::styled("logs  ", Style::default().fg(dim)),
            Span::styled("? ", Style::default().fg(cyan).bold()),
            Span::styled("help  ", Style::default().fg(dim)),
            Span::styled("q ", Style::default().fg(cyan).bold()),
            Span::styled("quit", Style::default().fg(dim)),
        ]
    };
    let line = Line::from(spans).alignment(ratatui::layout::Alignment::Right);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_footer(frame: &mut Frame, area: Rect) {
    let version = env!("CARGO_PKG_VERSION");
    let dim = Color::Rgb(80, 80, 80);
    let left = vec![Span::styled("~", Style::default().fg(dim))];
    let right = vec![Span::styled(version, Style::default().fg(dim))];

    let mut line = left;
    let left_len: usize = line.iter().map(|s| s.width()).sum();
    let right_len: usize = right.iter().map(|s| s.width()).sum();
    let space = area
        .width
        .saturating_sub(left_len as u16 + right_len as u16);
    if space > 0 {
        line.push(Span::raw(" ".repeat(space as usize)));
    }
    line.extend(right);
    frame.render_widget(Paragraph::new(Line::from(line)), area);
}

fn draw_logs(frame: &mut Frame, area: Rect, app: &App) {
    let visible = area.height.saturating_sub(2).max(1) as usize;
    let log_path = paths::service_log_path(ServiceKind::AtBridge.name());
    let lines = service::tail_lines(&log_path, LOG_TAIL);

    let green = Color::Rgb(16, 163, 127);
    let (start, end, scroll, max_scroll) = log_window_bounds(lines.len(), visible, app.log_scroll);
    let shown: Vec<Line> = if lines.is_empty() {
        vec![Line::from(
            "(no execution output yet; start the bridge service to view logs)",
        )]
    } else {
        lines[start..end]
            .iter()
            .map(|line| {
                if line.contains("ERROR") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Red)))
                } else if line.contains("WARN") {
                    Line::from(Span::styled(
                        line.clone(),
                        Style::default().fg(Color::Yellow),
                    ))
                } else if line.len() > 30
                    && line.chars().next().unwrap_or(' ').is_numeric()
                    && line.is_char_boundary(30)
                {
                    let (ts, rest) = line.split_at(30);
                    let rest_spans = if rest.contains("INFO") {
                        vec![
                            Span::styled(ts.to_string(), Style::default().dim()),
                            Span::styled("INFO", Style::default().fg(green).bold()),
                            Span::raw(rest.replace("INFO", "")),
                        ]
                    } else {
                        vec![
                            Span::styled(ts.to_string(), Style::default().dim()),
                            Span::raw(rest.to_string()),
                        ]
                    };
                    Line::from(rest_spans)
                } else {
                    Line::from(line.clone())
                }
            })
            .collect()
    };

    let title_base = " execution output ";

    let scroll_label = if max_scroll == 0 {
        "".to_string()
    } else if scroll == 0 {
        format!(" [newest · {} older] ", max_scroll)
    } else if scroll == max_scroll {
        format!(" [oldest · {}/{}] ", scroll, max_scroll)
    } else {
        format!(" [{}/{}] ", scroll, max_scroll)
    };

    let border_style = Style::default().fg(green);
    let title = format!(" {}{} ", title_base, scroll_label);
    let para = Paragraph::new(shown).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title),
    );
    frame.render_widget(para, area);
}

fn log_window_bounds(
    line_count: usize,
    visible: usize,
    requested_scroll: usize,
) -> (usize, usize, usize, usize) {
    if line_count == 0 {
        return (0, 0, 0, 0);
    }
    let visible = visible.max(1);
    let max_scroll = line_count.saturating_sub(visible);
    let scroll = requested_scroll.min(max_scroll);
    let end = line_count.saturating_sub(scroll);
    let start = end.saturating_sub(visible);
    (start, end, scroll, max_scroll)
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App, bridge: &ServiceStatus, sync_line: &str) {
    let blue = Color::Rgb(59, 130, 246);
    let orange = Color::Rgb(245, 158, 11);
    let dark_bg = Color::Rgb(25, 26, 27);
    let dim = Color::Rgb(120, 120, 120);

    let (status_text, is_active) = if bridge.running {
        let pid_str = bridge
            .pid
            .map(|p| format!(" (PID: {})", p))
            .unwrap_or_default();
        (format!("AtBridge ● ACTIVE{}", pid_str), true)
    } else {
        ("AtBridge ● OFFLINE".to_string(), false)
    };

    let sync_status = if is_active { "active" } else { "idle" };

    let placeholder = if area.width < 75 {
        "Ask anything... \"s: start · tab: logs · q: quit\""
    } else {
        "Ask anything... \"s to start bridge · tab to view logs · q to quit\""
    };

    let prompt_line = match app.mode {
        Mode::Command => {
            if app.input.is_empty() {
                Line::from(vec![
                    Span::styled(" ┃ ", Style::default().fg(blue).bold()),
                    Span::styled(" › ", Style::default().fg(blue).bold()),
                    Span::styled("Ask anything...", Style::default().fg(dim)),
                    Span::styled("█", Style::default().fg(blue)),
                ])
            } else {
                Line::from(vec![
                    Span::styled(" ┃ ", Style::default().fg(blue).bold()),
                    Span::styled(" › ", Style::default().fg(blue).bold()),
                    Span::raw(app.input.clone()),
                    Span::styled("█", Style::default().fg(blue)),
                ])
            }
        }
        Mode::Normal => Line::from(vec![
            Span::styled(" ┃ ", Style::default().fg(blue).bold()),
            Span::styled(placeholder, Style::default().fg(dim)),
        ]),
    };

    let details_line = Line::from(vec![
        Span::styled(" ┃ ", Style::default().fg(blue).bold()),
        Span::styled("Bridge", Style::default().fg(blue).bold()),
        Span::styled("  ·  ", Style::default().fg(dim)),
        Span::raw(status_text),
        Span::styled("  ·  ", Style::default().fg(dim)),
        Span::styled(sync_status, Style::default().fg(orange).bold()),
    ]);

    let sync_info_line = Line::from(vec![
        Span::styled(" ┃ ", Style::default().fg(blue).bold()),
        Span::styled(sync_line.to_string(), Style::default().fg(dim)),
    ]);

    let border_color = if app.mode == Mode::Command {
        blue
    } else {
        Color::Rgb(60, 60, 60)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .bg(dark_bg);

    let para = Paragraph::new(vec![prompt_line, details_line, sync_info_line]).block(block);

    frame.render_widget(para, area);
}

/// One-line summary of the most recent sync across configured devices, read
/// from the persisted per-device state files. Also the home page's hint about
/// what to do next when nothing is configured yet.
fn last_sync_summary() -> String {
    use crate::{adapters::config_file::JsonConfigStore, ports::config_store::ConfigStore};

    let path = paths::default_config_path();
    if !path.exists() {
        return "no config yet — type setup to connect your device and HRMS".to_string();
    }
    let Ok(cfg) = JsonConfigStore.load(&path) else {
        return "config invalid — type setup to reconfigure".to_string();
    };
    if cfg.devices.is_empty() {
        return "no devices configured — type setup to add one".to_string();
    }

    let mut latest: Option<crate::domain::SyncResult> = None;
    for device in &cfg.devices {
        if let Some(result) = crate::runtime::sync_state::load_last_result(&device.device_code) {
            let newer = latest
                .as_ref()
                .is_none_or(|cur| result.started_at > cur.started_at);
            if newer {
                latest = Some(result);
            }
        }
    }

    match latest {
        None => format!(
            "{} device(s) configured · no sync yet — press s to start the bridge",
            cfg.devices.len()
        ),
        Some(result) => {
            let ago = chrono::DateTime::parse_from_rfc3339(&result.started_at)
                .map(|t| {
                    let secs = (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds();
                    match secs {
                        s if s < 0 => "just now".to_string(),
                        s if s < 60 => format!("{s}s ago"),
                        s if s < 3600 => format!("{}m ago", s / 60),
                        s if s < 86400 => format!("{}h ago", s / 3600),
                        s => format!("{}d ago", s / 86400),
                    }
                })
                .unwrap_or_else(|_| "time unknown".to_string());
            let mark = if result.ok { "✓" } else { "✗" };
            format!(
                "last sync {mark} {} · pulled {} · forwarded {} · {ago}",
                result.device_code, result.pulled, result.forwarded
            )
        }
    }
}

fn bridge_status(rows: Vec<ServiceStatus>) -> ServiceStatus {
    rows.into_iter()
        .find(|row| row.name == "bridge")
        .unwrap_or(ServiceStatus {
            name: "bridge".to_string(),
            kind: ServiceKind::AtBridge,
            running: false,
            pid: None,
            port: None,
            url: None,
            uptime_secs: None,
        })
}

#[cfg(test)]
mod tests {
    use super::{log_window_bounds, should_run_attached, split_command_line};

    #[test]
    fn command_line_splitter_keeps_quoted_paths_together() {
        let words =
            split_command_line("bridge config show --path \"/tmp/my config.json\"").expect("split");
        assert_eq!(
            words,
            ["bridge", "config", "show", "--path", "/tmp/my config.json"]
        );
    }

    #[test]
    fn interactive_commands_are_attached_to_the_real_terminal() {
        assert!(should_run_attached(&["uninstall".into()]));
        assert!(should_run_attached(&["update".into()]));
        assert!(!should_run_attached(&["update".into(), "--check".into()]));
    }

    #[test]
    fn log_window_defaults_to_newest_lines() {
        assert_eq!(log_window_bounds(100, 10, 0), (90, 100, 0, 90));
        assert_eq!(log_window_bounds(5, 10, 0), (0, 5, 0, 0));
    }

    #[test]
    fn log_window_scrolls_older_and_clamps() {
        assert_eq!(log_window_bounds(100, 10, 7), (83, 93, 7, 90));
        assert_eq!(log_window_bounds(100, 10, usize::MAX), (0, 10, 90, 90));
    }
}
