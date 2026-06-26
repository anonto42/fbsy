//! Live full-screen service dashboard (`fbsy dashboard`).
//!
//! A ratatui front-end over the same registry/process core that powers
//! `fbsy show`/`run`/`close`. It auto-refreshes, shows a live log pane, and
//! offers two ways to drive services: single-key shortcuts and a `:command`
//! bar that accepts the full service-management vocabulary.

use std::{io::IsTerminal, time::Duration};

use anyhow::Result;
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
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

struct App {
    selected: usize,
    show_logs: bool,
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
            mode: Mode::Normal,
            input: String::new(),
            status: "ready".to_string(),
            quit: false,
        }
    }

    fn selected_kind(&self) -> ServiceKind {
        ServiceKind::all()[self.selected]
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
    let count = ServiceKind::all().len();

    while !app.quit {
        let rows = service::snapshot();
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
            Mode::Normal => handle_normal_key(&mut app, key.code, count),
        }
    }
    Ok(())
}

// ── Key handling ──────────────────────────────────────────────────────────────

fn handle_normal_key(app: &mut App, code: KeyCode, count: usize) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
        KeyCode::Char(':') => {
            app.mode = Mode::Command;
            app.input.clear();
        }
        KeyCode::Up | KeyCode::Char('k') => app.selected = app.selected.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => {
            if app.selected + 1 < count {
                app.selected += 1;
            }
        }
        KeyCode::Char('l') => app.show_logs = !app.show_logs,
        KeyCode::Char('s') => run_action(app, Action::Start(app.selected_kind())),
        KeyCode::Char('x') => run_action(app, Action::Stop(app.selected_kind())),
        KeyCode::Char('r') => run_action(app, Action::Restart(app.selected_kind())),
        KeyCode::Char('y') => run_action(app, Action::Sync(None)),
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
    Stop(ServiceKind),
    Restart(ServiceKind),
    Sync(Option<String>),
}

fn run_action(app: &mut App, action: Action) {
    app.status = match action {
        Action::Start(kind) => match service::default_start(kind) {
            Ok(pid) => format!("started {} (pid {pid})", kind.name()),
            Err(e) => format!("start {}: {e}", kind.name()),
        },
        Action::Stop(kind) => match service::stop_service(kind) {
            Ok(true) => format!("stopped {}", kind.name()),
            Ok(false) => format!("{} was not running", kind.name()),
            Err(e) => format!("stop {}: {e}", kind.name()),
        },
        Action::Restart(kind) => {
            let _ = service::stop_service(kind);
            std::thread::sleep(Duration::from_millis(150));
            match service::default_start(kind) {
                Ok(pid) => format!("restarted {} (pid {pid})", kind.name()),
                Err(e) => format!("restart {}: {e}", kind.name()),
            }
        }
        Action::Sync(device) => match sync_once::run_summary(None, device) {
            Ok(summary) => summary,
            Err(e) => format!("sync failed: {e}"),
        },
    };
}

/// Parse and run a `:command` line. Vocabulary:
///   start|stop|restart <service> · sync [deviceCode] · logs <service>
///   select <service> · help · quit
fn execute_command(app: &mut App, line: &str) {
    let mut parts = line.split_whitespace();
    let verb = parts.next().unwrap_or("");
    let arg = parts.next();

    let resolve =
        |name: Option<&str>| -> Option<ServiceKind> { name.and_then(ServiceKind::from_name) };

    match verb {
        "start" => match resolve(arg) {
            Some(k) => run_action(app, Action::Start(k)),
            None => app.status = "usage: start <bridge|zkteco|hrms>".to_string(),
        },
        "stop" => match resolve(arg) {
            Some(k) => run_action(app, Action::Stop(k)),
            None => app.status = "usage: stop <bridge|zkteco|hrms>".to_string(),
        },
        "restart" => match resolve(arg) {
            Some(k) => run_action(app, Action::Restart(k)),
            None => app.status = "usage: restart <bridge|zkteco|hrms>".to_string(),
        },
        "sync" => run_action(app, Action::Sync(arg.map(str::to_string))),
        "logs" => match resolve(arg) {
            Some(k) => {
                app.selected = ServiceKind::all().iter().position(|x| *x == k).unwrap_or(0);
                app.show_logs = true;
                app.status = format!("showing logs: {}", k.name());
            }
            None => app.status = "usage: logs <bridge|zkteco|hrms>".to_string(),
        },
        "select" => match resolve(arg) {
            Some(k) => {
                app.selected = ServiceKind::all().iter().position(|x| *x == k).unwrap_or(0);
                app.status = format!("selected {}", k.name());
            }
            None => app.status = "usage: select <bridge|zkteco|hrms>".to_string(),
        },
        "help" => {
            app.status =
                "commands: start|stop|restart <svc> · sync [code] · logs <svc> · quit".to_string()
        }
        "quit" | "q" | "exit" => app.quit = true,
        other => app.status = format!("unknown command '{other}' (try: help)"),
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn draw(frame: &mut Frame, app: &App, rows: &[ServiceStatus]) {
    let log_constraint = if app.show_logs {
        Constraint::Min(5)
    } else {
        Constraint::Length(0)
    };
    let areas = Layout::vertical([
        Constraint::Length(3), // title
        Constraint::Length(7), // service table
        log_constraint,        // log pane
        Constraint::Length(4), // command palette
        Constraint::Length(1), // status / input line
    ])
    .split(frame.area());

    draw_title(frame, areas[0]);
    draw_table(frame, areas[1], app, rows);
    if app.show_logs {
        draw_logs(frame, areas[2], app, rows);
    }
    draw_palette(frame, areas[3]);
    draw_status(frame, areas[4], app);
}

fn draw_title(frame: &mut Frame, area: Rect) {
    let title = Paragraph::new(Line::from(vec![
        "fbsy".bold().cyan(),
        "  service dashboard".into(),
        "   —  : for command, q to quit".dim(),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, area);
}

fn draw_table(frame: &mut Frame, area: Rect, app: &App, rows: &[ServiceStatus]) {
    let header = Row::new(["SERVICE", "STATUS", "PID", "PORT", "UPTIME", "ADDRESS"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let body: Vec<Row> = rows
        .iter()
        .map(|r| {
            let (status_text, status_color) = if r.running {
                ("running", Color::Green)
            } else {
                ("stopped", Color::Red)
            };
            let pid = r.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into());
            let port = r.port.map(|p| p.to_string()).unwrap_or_else(|| "-".into());
            let uptime = r
                .uptime_secs
                .map(service::format_uptime_secs)
                .unwrap_or_else(|| "-".into());
            // Show the live address when running; the static description otherwise.
            let address = if r.running {
                r.url
                    .clone()
                    .unwrap_or_else(|| r.kind.description().to_string())
            } else {
                r.kind.description().to_string()
            };
            Row::new(vec![
                Cell::from(r.kind.name()),
                Cell::from(status_text).style(Style::default().fg(status_color)),
                Cell::from(pid),
                Cell::from(port),
                Cell::from(uptime),
                Cell::from(address),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(10),
        Constraint::Length(9),
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Min(22),
    ];
    let table = Table::new(body, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" services "))
        // A bold + cyan highlight + a ▶ marker indicates selection without
        // inverting the per-cell status colors.
        .row_highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut state = TableState::default();
    state.select(Some(app.selected));
    frame.render_stateful_widget(table, area, &mut state);
}

fn draw_logs(frame: &mut Frame, area: Rect, app: &App, rows: &[ServiceStatus]) {
    let kind = app.selected_kind();
    let running = rows.get(app.selected).map(|r| r.running).unwrap_or(false);
    let log_path = paths::service_log_path(kind.name());

    let visible = area.height.saturating_sub(2) as usize;
    let lines = service::tail_lines(&log_path, LOG_TAIL);
    let shown: Vec<Line> = lines
        .iter()
        .rev()
        .take(visible.max(1))
        .rev()
        .map(|l| Line::from(l.clone()))
        .collect();

    let title = format!(
        " logs: {} ({}) ",
        kind.name(),
        if running { "running" } else { "stopped" }
    );
    let para = Paragraph::new(shown).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(para, area);
}

fn draw_palette(frame: &mut Frame, area: Rect) {
    let key = |k: &str, d: &str| -> Vec<Span<'static>> {
        vec![
            Span::styled(k.to_string(), Style::default().fg(Color::Cyan).bold()),
            Span::raw(format!(" {d}   ")),
        ]
    };
    let mut line1 = Vec::new();
    line1.extend(key("↑/↓", "select"));
    line1.extend(key("s", "start"));
    line1.extend(key("x", "stop"));
    line1.extend(key("r", "restart"));
    line1.extend(key("y", "sync"));
    line1.extend(key("l", "logs"));
    line1.extend(key("q", "quit"));

    let line2 = vec![
        Span::styled(":", Style::default().fg(Color::Yellow).bold()),
        Span::raw(" command  —  "),
        Span::styled(
            "start|stop|restart <svc> · sync [code] · logs <svc> · select <svc> · help",
            Style::default().dim(),
        ),
    ];

    let para = Paragraph::new(vec![Line::from(line1), Line::from(line2)])
        .block(Block::default().borders(Borders::ALL).title(" commands "));
    frame.render_widget(para, area);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let line = match app.mode {
        Mode::Command => Line::from(vec![
            Span::styled(":", Style::default().fg(Color::Yellow).bold()),
            Span::raw(app.input.clone()),
            Span::styled("█", Style::default().fg(Color::Yellow)),
        ]),
        Mode::Normal => Line::from(vec![
            Span::styled("status ", Style::default().dim()),
            Span::styled(app.status.clone(), Style::default().fg(Color::Yellow)),
        ]),
    };
    frame.render_widget(Paragraph::new(line), area);
}
