//! Live full-screen service dashboard (`fbsy dashboard`).
//!
//! A ratatui front-end over the same registry/process core that powers
//! `fbsy show`/`run`/`close`. It auto-refreshes, shows a live log pane, and
//! offers two ways to drive services: single-key shortcuts and a `:command`
//! bar that accepts the full service-management vocabulary.

use std::{
    io::{self, IsTerminal, Write},
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result};
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap},
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
enum Mode {
    Normal,
    Command,
}

/// Which pane the arrow keys drive: the service table or the log view.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Focus {
    Table,
    Logs,
}

/// Captured output of a passthrough CLI command, shown in a scrollable overlay.
struct OutputView {
    title: String,
    lines: Vec<String>,
    scroll: usize,
}

struct App {
    selected: usize,
    show_logs: bool,
    all_logs: bool,
    log_scroll: usize,
    focus: Focus,
    show_help: bool,
    help_scroll: usize,
    /// Some(view) while a captured CLI command's output overlay is open.
    output: Option<OutputView>,
    /// Set when a `:command` must run attached to the real terminal (interactive
    /// commands like `bridge config setup`); performed by the event loop, which
    /// owns the terminal so it can suspend and resume the TUI.
    pending_attach: Option<Vec<String>>,
    mode: Mode,
    input: String,
    status: String,
    quit: bool,
}

impl App {
    fn new() -> Self {
        Self {
            selected: 0,
            show_logs: true,
            all_logs: false,
            log_scroll: 0,
            focus: Focus::Table,
            show_help: false,
            help_scroll: 0,
            output: None,
            pending_attach: None,
            mode: Mode::Normal,
            input: String::new(),
            status: "press ? for help · : for commands".to_string(),
            quit: false,
        }
    }
}

/// Run the dashboard. Requires an interactive terminal.
pub fn run() -> Result<()> {
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        println!("fbsy dashboard needs an interactive terminal.");
        println!("Use `fbsy show` for a one-shot snapshot instead.");
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
        let rows = dashboard_rows(service::snapshot());
        if app.selected >= rows.len() {
            app.selected = rows.len().saturating_sub(1);
        }
        terminal.draw(|frame| draw(frame, &app, &rows))?;

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
            Mode::Normal => handle_normal_key(&mut app, key.code, &rows),
        }

        // An interactive command must own the real terminal: drop the TUI, run it
        // attached, then re-enter the alternate screen.
        if let Some(args) = app.pending_attach.take() {
            run_attached(terminal, &args, &mut app.status);
        }
    }
    Ok(())
}

// ── Key handling ──────────────────────────────────────────────────────────────

fn handle_normal_key(app: &mut App, code: KeyCode, rows: &[ServiceStatus]) {
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
    // While the help overlay is open, allow scrolling or close it.
    if app.show_help {
        match code {
            KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Esc => {
                app.show_help = false;
                app.help_scroll = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.help_scroll = app.help_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.help_scroll = app.help_scroll.saturating_add(1);
            }
            KeyCode::PageUp => {
                app.help_scroll = app.help_scroll.saturating_sub(10);
            }
            KeyCode::PageDown => {
                app.help_scroll = app.help_scroll.saturating_add(10);
            }
            KeyCode::Home => app.help_scroll = 0,
            KeyCode::End => app.help_scroll = usize::MAX,
            _ => {}
        }
        return;
    }
    // Keys handled the same regardless of which pane is focused.
    match code {
        KeyCode::Char('q') => {
            app.quit = true;
            return;
        }
        KeyCode::Char('?') => {
            app.show_help = true;
            return;
        }
        KeyCode::Char(':') => {
            app.mode = Mode::Command;
            app.input.clear();
            return;
        }
        KeyCode::Tab | KeyCode::BackTab => {
            app.focus = match app.focus {
                Focus::Table => {
                    app.show_logs = true;
                    Focus::Logs
                }
                Focus::Logs => Focus::Table,
            };
            app.status = focus_hint(app.focus);
            return;
        }
        KeyCode::Char('l') => {
            app.show_logs = !app.show_logs;
            app.focus = if app.show_logs {
                Focus::Logs
            } else {
                Focus::Table
            };
            app.log_scroll = 0;
            return;
        }
        KeyCode::Char('a') => {
            app.all_logs = !app.all_logs;
            app.show_logs = true;
            app.focus = Focus::Logs;
            app.log_scroll = 0;
            app.status = if app.all_logs {
                "logs: all running instances (↑/↓ scroll, Tab/Esc back to table)".to_string()
            } else {
                "logs: selected instance".to_string()
            };
            return;
        }
        KeyCode::Char('s') => {
            if let Some(row) = rows.get(app.selected) {
                run_action(app, Action::Start(row.kind));
            }
            return;
        }
        KeyCode::Char('x') => {
            if let Some(row) = rows.get(app.selected) {
                run_action(app, Action::Stop(row.name.clone()));
            }
            return;
        }
        KeyCode::Char('r') => {
            if let Some(row) = rows.get(app.selected) {
                run_action(app, Action::Restart(row.name.clone(), row.kind));
            }
            return;
        }
        KeyCode::Char('y') => {
            run_action(app, Action::Sync(None));
            return;
        }
        _ => {}
    }

    // Pane-specific navigation: arrows scroll the focused pane.
    match app.focus {
        Focus::Logs => match code {
            KeyCode::Esc => {
                app.focus = Focus::Table;
                app.status = focus_hint(Focus::Table);
            }
            KeyCode::Up | KeyCode::Char('k') => app.log_scroll = app.log_scroll.saturating_add(1),
            KeyCode::Down | KeyCode::Char('j') => app.log_scroll = app.log_scroll.saturating_sub(1),
            KeyCode::PageUp => app.log_scroll = app.log_scroll.saturating_add(10),
            KeyCode::PageDown => app.log_scroll = app.log_scroll.saturating_sub(10),
            KeyCode::Home => app.log_scroll = usize::MAX, // oldest
            KeyCode::End => app.log_scroll = 0,           // newest
            _ => {}
        },
        Focus::Table => match code {
            KeyCode::Esc => app.quit = true,
            KeyCode::Up | KeyCode::Char('k') => {
                app.selected = app.selected.saturating_sub(1);
                app.log_scroll = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if app.selected + 1 < rows.len() {
                    app.selected += 1;
                    app.log_scroll = 0;
                }
            }
            // PgUp/PgDn still scroll logs even from the table, for convenience.
            KeyCode::PageUp => {
                app.show_logs = true;
                app.log_scroll = app.log_scroll.saturating_add(10);
            }
            KeyCode::PageDown => app.log_scroll = app.log_scroll.saturating_sub(10),
            _ => {}
        },
    }
}

/// Status-line hint describing what the arrow keys do for the given focus.
fn focus_hint(focus: Focus) -> String {
    match focus {
        Focus::Table => "focus: table (↑/↓ select · Tab → logs)".to_string(),
        Focus::Logs => "focus: logs (↑/↓ scroll · Tab/Esc → table)".to_string(),
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
/// The dashboard keeps a few TUI-native conveniences (`select`, `logs all`,
/// `restart`) and translates friendly aliases (`start`, `stop`, `sync`) into
/// normal CLI commands. Everything else is passed through to this same `fbsy`
/// binary, so the dashboard command bar can use the full CLI surface.
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
        app.status = "type a command, for example: show".to_string();
        return;
    }

    let rows = dashboard_rows(service::snapshot());
    match args[0].as_str() {
        "help" | "?" => {
            app.show_help = true;
            return;
        }
        "quit" | "q" | "exit" => {
            app.quit = true;
            return;
        }
        "select" => {
            match args
                .get(1)
                .and_then(|name| row_index_by_name_or_kind(&rows, name))
            {
                Some(index) => {
                    app.selected = index;
                    app.log_scroll = 0;
                    app.status = format!("selected {}", rows[index].name);
                }
                None => app.status = "usage: select <instance>|<kind>".to_string(),
            }
            return;
        }
        "logs" if args.get(1).is_some_and(|arg| arg == "all") => {
            app.all_logs = true;
            app.show_logs = true;
            app.log_scroll = 0;
            app.status = "showing logs: all running instances".to_string();
            return;
        }
        "restart" => {
            match args
                .get(1)
                .and_then(|name| row_by_name_or_kind(&rows, name))
            {
                Some(row) => run_action(app, Action::Restart(row.name.clone(), row.kind)),
                None => app.status = "usage: restart <instance>".to_string(),
            }
            return;
        }
        "dashboard" => {
            app.status = "already inside fbsy dashboard".to_string();
            return;
        }
        _ => {}
    }

    let args = expand_dashboard_alias(args);
    if should_run_attached(&args) {
        app.pending_attach = Some(args);
        app.status = "leaving dashboard temporarily for an interactive command".to_string();
    } else {
        app.output = Some(run_captured(&args));
        app.status = "command output opened; Esc closes it".to_string();
    }
}

fn expand_dashboard_alias(mut args: Vec<String>) -> Vec<String> {
    match args[0].as_str() {
        "start" => {
            args[0] = "run".to_string();
            args
        }
        "stop" => {
            args[0] = "close".to_string();
            args
        }
        "sync" => {
            let mut out = vec![
                "bridge".to_string(),
                "sync".to_string(),
                "--once".to_string(),
            ];
            match args.get(1) {
                Some(device) if !device.starts_with('-') => {
                    out.push("--device".to_string());
                    out.push(device.clone());
                    out.extend(args.into_iter().skip(2));
                }
                Some(_) => out.extend(args.into_iter().skip(1)),
                None => {}
            }
            out
        }
        "doctor" | "config" | "devices" | "webhook" => {
            let mut out = vec!["bridge".to_string()];
            out.extend(args);
            out
        }
        "setup" => vec![
            "bridge".to_string(),
            "config".to_string(),
            "setup".to_string(),
        ],
        "once" => {
            let mut out = vec![
                "bridge".to_string(),
                "sync".to_string(),
                "--once".to_string(),
            ];
            out.extend(args.into_iter().skip(1));
            out
        }
        _ => args,
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
        [cmd, service, ..] if cmd == "run" && service == "bridge" => true,
        [cmd, sub, ..] if cmd == "bridge" && sub == "run" => true,
        [cmd, sub, leaf, ..] if cmd == "bridge" && sub == "config" && leaf == "setup" => true,
        [cmd, ..] if cmd == "logs" && has_flag("-f", "--follow") => true,
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
    args: &[String],
    status_text: &mut String,
) {
    ratatui::restore();
    println!();
    println!("$ fbsy {}", display_args(args));
    println!();

    let status = std::env::current_exe()
        .context("locate current executable")
        .and_then(|exe| {
            Command::new(exe)
                .args(args)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .status()
                .context("run attached command")
        });

    match status {
        Ok(status) if status.success() => {
            *status_text = format!("fbsy {} completed", display_args(args));
        }
        Ok(status) => {
            *status_text = format!("fbsy {} exited with {status}", display_args(args));
        }
        Err(err) => {
            *status_text = format!("could not run fbsy {}: {err}", display_args(args));
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

fn draw(frame: &mut Frame, app: &App, rows: &[ServiceStatus]) {
    let area = frame.area();
    if area.width < 80 || area.height < 20 {
        let msg = format!(
            "Terminal too small ({}x{}). Minimum is 80x20. Please resize.",
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

    let log_constraint = if app.show_logs {
        Constraint::Min(8)
    } else {
        Constraint::Length(0)
    };

    let table_height = (rows.len() as u16 + 3).max(5);

    let areas = Layout::vertical([
        Constraint::Length(3),            // title
        Constraint::Length(table_height), // service table (dynamic height)
        log_constraint,                   // log pane
        Constraint::Length(4),            // command palette
        Constraint::Length(1),            // status / input line
    ])
    .split(frame.area());

    draw_title(frame, areas[0], rows);
    draw_table(frame, areas[1], app, rows);
    if app.show_logs {
        draw_logs(frame, areas[2], app, rows);
    }
    draw_palette(frame, areas[3]);
    draw_status(frame, areas[4], app);

    // The help overlay floats above everything else.
    if let Some(view) = &app.output {
        draw_output(frame, frame.area(), view);
    }
    if app.show_help {
        draw_help(frame, frame.area(), app);
    }
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
        Line::from(vec![Span::styled(" Single-key shortcuts", yellow)]),
        row("Tab", "switch focus: service table ⇄ log pane"),
        row("↑/↓ or j/k", "table focus: select · log focus: scroll"),
        row("s", "start selected service (default name + port)"),
        row("x", "stop selected instance"),
        row("r", "restart selected instance"),
        row("y", "run a one-off sync now"),
        row("l", "toggle the log pane (and focus it)"),
        row("a", "logs from ALL running instances, time-merged"),
        row("PgUp / PgDn", "scroll log pane older / newer"),
        row("Home / End", "jump to oldest / newest log lines"),
        row("Esc", "log focus → table · table focus → quit"),
        row("? / q", "this help / quit"),
        separator(),
        Line::from(vec![Span::styled(
            " Command bar passthrough  (press : then type)",
            yellow,
        )]),
        Line::from(vec![
            "  Type any normal CLI command ".into(),
            Span::styled("without", yellow),
            " the `fbsy` prefix.".into(),
        ]),
        row("show", "same as `fbsy show`"),
        row("scan --all-ports", "discover network services & devices"),
        row("bridge doctor --json", "captures command output here"),
        row(
            "bridge devices info CODE --users",
            "deep diagnostics also work",
        ),
        row(
            "bridge config setup",
            "suspends TUI for the interactive wizard",
        ),
        separator(),
        Line::from(vec![Span::styled(" Dashboard aliases", yellow)]),
        row("start <kind> [flags]", "alias for `run <kind> [flags]`"),
        row("stop <instance>", "alias for `close <instance>`"),
        row("restart <instance>", "dashboard-only restart helper"),
        row(
            "sync [deviceCode]",
            "alias for `bridge sync --once [--device CODE]`",
        ),
        row("logs all", "dashboard-only combined log view"),
        row("select <instance>", "move the highlight"),
        separator(),
        Line::from(vec![Span::styled(" Running multiple instances", yellow)]),
        Line::from(vec![
            "  ".into(),
            Span::styled("start zkteco --name dev1 --port 4370", cyan),
        ]),
        Line::from(vec![
            "  ".into(),
            Span::styled("start zkteco --name dev2 --port 4371", cyan),
            "   ".into(),
            Span::styled("← a 2nd mock device", dim),
        ]),
        Line::from(vec![
            "  ".into(),
            Span::styled("start bridge --config /path/config.json", cyan),
        ]),
        Line::from(vec![
            "  ".into(),
            Span::styled("start scanner --interval 300", cyan),
        ]),
    ];

    let visible = area.height.saturating_sub(2) as usize;
    let max_scroll = lines.len().saturating_sub(visible);
    let scroll = app.help_scroll.min(max_scroll).min(u16::MAX as usize) as u16;

    let title = if max_scroll > 0 {
        format!(" help ({}/{max_scroll}) — ↑/↓ scroll · Esc close ", scroll)
    } else {
        " help — Esc close ".to_string()
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

fn draw_title(frame: &mut Frame, area: Rect, rows: &[ServiceStatus]) {
    let version = env!("CARGO_PKG_VERSION");
    let running = rows.iter().filter(|r| r.running).count();
    let ip = crate::support::network::lan_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "127.0.0.1".to_string());

    let left = vec![
        Span::styled(" fbsy v", Style::default().fg(Color::Cyan).bold()),
        Span::styled(version, Style::default().fg(Color::Cyan).bold()),
        Span::raw("  ·  "),
        Span::styled(format!("{running} running"), Style::default().bold()),
        Span::raw("  ·  "),
        Span::raw(ip),
    ];
    let right = vec![Span::styled(
        " : commands · ? help · q quit ",
        Style::default().dim(),
    )];

    let mut line = left;
    // Simple right-alignment padding logic
    let left_len: usize = line.iter().map(|s| s.width()).sum();
    let right_len: usize = right.iter().map(|s| s.width()).sum();
    let space = area
        .width
        .saturating_sub(left_len as u16 + right_len as u16 + 2); // +2 for borders
    if space > 0 {
        line.push(Span::raw(" ".repeat(space as usize)));
    }
    line.extend(right);

    let title = Paragraph::new(Line::from(line)).block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, area);
}

fn draw_table(frame: &mut Frame, area: Rect, app: &App, rows: &[ServiceStatus]) {
    let header = Row::new(["SERVICE", "STATUS", "PID", "PORT", "UPTIME", "ADDRESS"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let body: Vec<Row> = rows
        .iter()
        .map(|r| {
            let (status_text, status_color) = if r.running {
                ("● running", Color::Green)
            } else {
                ("● stopped", Color::Red)
            };
            let pid = r.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into());
            let port = r.port.map(|p| p.to_string()).unwrap_or_else(|| "-".into());
            let uptime = r
                .uptime_secs
                .map(service::format_uptime_secs)
                .unwrap_or_else(|| "-".into());
            let address = if r.running {
                r.url
                    .clone()
                    .unwrap_or_else(|| r.kind.description().to_string())
            } else {
                r.kind.description().to_string()
            };
            Row::new(vec![
                Cell::from(if r.name == r.kind.name() {
                    r.kind.name().to_string()
                } else {
                    format!("{} ({})", r.name, r.kind.name())
                }),
                Cell::from(status_text).style(Style::default().fg(status_color)),
                Cell::from(pid),
                Cell::from(port),
                Cell::from(uptime),
                Cell::from(address),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(10),
        Constraint::Min(22),
    ];

    let focused = app.focus == Focus::Table;
    let (title, border_style) = if focused {
        (" services (focus) ", Style::default().fg(Color::Cyan))
    } else {
        (" services ", Style::default().dim())
    };
    let table = Table::new(body, widths)
        .header(header)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(border_style)
                .title(title),
        )
        .row_highlight_style(
            Style::default()
                .bg(if focused {
                    Color::DarkGray
                } else {
                    Color::Reset
                })
                .fg(if focused {
                    Color::Cyan
                } else {
                    Color::DarkGray
                })
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(if focused { "▶ " } else { "  " });

    let mut state = TableState::default();
    state.select(Some(app.selected));
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_logs(frame: &mut Frame, area: Rect, app: &App, rows: &[ServiceStatus]) {
    let visible = area.height.saturating_sub(2).max(1) as usize;
    let lines = if app.all_logs {
        service::tail_all_running(LOG_TAIL)
    } else {
        let name = rows
            .get(app.selected)
            .map(|r| r.name.as_str())
            .unwrap_or(ServiceKind::AtBridge.name());
        let log_path = paths::service_log_path(name);
        service::tail_lines(&log_path, LOG_TAIL)
    };

    let (start, end, scroll, max_scroll) = log_window_bounds(lines.len(), visible, app.log_scroll);
    let shown: Vec<Line> = if lines.is_empty() {
        vec![Line::from(
            "(no log output yet; start a service or run `fbsy logs <instance>`)",
        )]
    } else {
        lines[start..end]
            .iter()
            .map(|line| {
                // Dim timestamps, colorize errors
                if line.contains("ERROR") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Red)))
                } else if line.contains("WARN") {
                    Line::from(Span::styled(
                        line.clone(),
                        Style::default().fg(Color::Yellow),
                    ))
                } else if line.len() > 30 && line.chars().next().unwrap_or(' ').is_numeric() {
                    let (ts, rest) = line.split_at(30);
                    Line::from(vec![
                        Span::styled(ts.to_string(), Style::default().dim()),
                        Span::raw(rest.to_string()),
                    ])
                } else {
                    Line::from(line.clone())
                }
            })
            .collect()
    };

    let title_base = if app.all_logs {
        "logs: all running instances".to_string()
    } else {
        let row = rows.get(app.selected);
        format!(
            "logs: {} ({})",
            row.map(|r| r.name.as_str()).unwrap_or("bridge"),
            if row.map(|r| r.running).unwrap_or(false) {
                "running"
            } else {
                "stopped"
            }
        )
    };

    let scroll_label = if max_scroll == 0 {
        "".to_string()
    } else if scroll == 0 {
        format!(" ▼ newest · {} older ", max_scroll)
    } else if scroll == max_scroll {
        format!(" ▲ oldest [{scroll}/{max_scroll}] ")
    } else {
        format!(" [{scroll}/{max_scroll}] ")
    };

    let focused = app.focus == Focus::Logs;
    let focus_tag = if focused { " (focus)" } else { "" };
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().dim()
    };
    let title = format!(" {title_base}{focus_tag}{scroll_label} ");
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

fn draw_palette(frame: &mut Frame, area: Rect) {
    let key = |k: &str, d: &str| -> Vec<Span<'static>> {
        vec![
            Span::styled(format!(" {k:<3} "), Style::default().fg(Color::Cyan).bold()),
            Span::raw(format!("{d:<14}")),
        ]
    };
    let mut line1 = Vec::new();
    line1.extend(key("Tab", "focus"));
    line1.extend(key("↑/↓", "scroll"));
    line1.extend(key("s", "start"));
    line1.extend(key("x", "stop"));
    line1.extend(key("r", "restart"));
    line1.extend(key("y", "sync"));

    let mut line2 = Vec::new();
    line2.extend(key("l", "logs"));
    line2.extend(key("a", "all logs"));
    line2.extend(key("?", "help"));
    line2.extend(key("q", "quit"));
    line2.extend(vec![
        Span::styled(" : ", Style::default().fg(Color::Yellow).bold()),
        Span::styled(
            "type any CLI command without `fbsy` (e.g. `show`, `scan`)",
            Style::default().dim(),
        ),
    ]);

    let para = Paragraph::new(vec![Line::from(line1), Line::from(line2)])
        .block(Block::default().borders(Borders::ALL).title(" commands "));
    frame.render_widget(para, area);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let (mode_str, mode_color) = match app.mode {
        Mode::Normal => (" NORMAL ", Color::DarkGray),
        Mode::Command => (" COMMAND ", Color::Yellow),
    };

    let line = match app.mode {
        Mode::Command => Line::from(vec![
            Span::styled(
                mode_str,
                Style::default().bg(mode_color).fg(Color::Black).bold(),
            ),
            Span::styled(" :", Style::default().fg(Color::Yellow).bold()),
            Span::raw(app.input.clone()),
            Span::styled("█", Style::default().fg(Color::Yellow)),
        ]),
        Mode::Normal => Line::from(vec![
            Span::styled(
                mode_str,
                Style::default().bg(mode_color).fg(Color::White).bold(),
            ),
            Span::styled("  ", Style::default().dim()),
            Span::styled(app.status.clone(), Style::default().fg(Color::Yellow)),
        ]),
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn dashboard_rows(mut rows: Vec<ServiceStatus>) -> Vec<ServiceStatus> {
    for kind in ServiceKind::all() {
        if rows.iter().any(|row| row.name == kind.name()) {
            continue;
        }
        rows.push(ServiceStatus {
            name: kind.name().to_string(),
            kind,
            running: false,
            pid: None,
            port: None,
            url: None,
            uptime_secs: None,
        });
    }
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows.sort_by_key(|row| row.kind as u8);
    rows
}

fn row_index_by_name_or_kind(rows: &[ServiceStatus], name: &str) -> Option<usize> {
    rows.iter()
        .position(|row| row.name == name)
        .or_else(|| rows.iter().position(|row| row.kind.name() == name))
}

fn row_by_name_or_kind<'a>(rows: &'a [ServiceStatus], name: &str) -> Option<&'a ServiceStatus> {
    row_index_by_name_or_kind(rows, name).and_then(|index| rows.get(index))
}

#[cfg(test)]
mod tests {
    use super::{
        expand_dashboard_alias, log_window_bounds, should_run_attached, split_command_line,
    };

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
    fn command_aliases_expand_to_real_cli_commands() {
        assert_eq!(
            expand_dashboard_alias(vec![
                "start".into(),
                "zkteco".into(),
                "--name".into(),
                "dev1".into()
            ]),
            ["run", "zkteco", "--name", "dev1"]
        );
        assert_eq!(
            expand_dashboard_alias(vec!["sync".into(), "GATE-01".into()]),
            ["bridge", "sync", "--once", "--device", "GATE-01"]
        );
        assert_eq!(
            expand_dashboard_alias(vec!["doctor".into(), "--json".into()]),
            ["bridge", "doctor", "--json"]
        );
    }

    #[test]
    fn interactive_commands_are_attached_to_the_real_terminal() {
        assert!(should_run_attached(&[
            "bridge".into(),
            "config".into(),
            "setup".into()
        ]));
        assert!(should_run_attached(&["update".into()]));
        assert!(!should_run_attached(&["update".into(), "--check".into()]));
        assert!(!should_run_attached(&["show".into()]));
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
