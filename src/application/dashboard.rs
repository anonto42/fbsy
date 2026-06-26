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
    all_logs: bool,
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
            mode: Mode::Normal,
            input: String::new(),
            status: "ready".to_string(),
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
    }
    Ok(())
}

// ── Key handling ──────────────────────────────────────────────────────────────

fn handle_normal_key(app: &mut App, code: KeyCode, rows: &[ServiceStatus]) {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
        KeyCode::Char(':') => {
            app.mode = Mode::Command;
            app.input.clear();
        }
        KeyCode::Up | KeyCode::Char('k') => app.selected = app.selected.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => {
            if app.selected + 1 < rows.len() {
                app.selected += 1;
            }
        }
        KeyCode::Char('l') => app.show_logs = !app.show_logs,
        KeyCode::Char('a') => {
            app.all_logs = !app.all_logs;
            app.show_logs = true;
            app.status = if app.all_logs {
                "showing logs from all running instances".to_string()
            } else {
                "showing selected instance logs".to_string()
            };
        }
        KeyCode::Char('s') => {
            if let Some(row) = rows.get(app.selected) {
                run_action(app, Action::Start(row.kind));
            }
        }
        KeyCode::Char('x') => {
            if let Some(row) = rows.get(app.selected) {
                run_action(app, Action::Stop(row.name.clone()));
            }
        }
        KeyCode::Char('r') => {
            if let Some(row) = rows.get(app.selected) {
                run_action(app, Action::Restart(row.name.clone(), row.kind));
            }
        }
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

/// Parse and run a `:command` line. Vocabulary:
///   start <kind> · stop|restart <instance> · sync [deviceCode]
///   logs <instance>|all · select <instance>|<kind> · help · quit
fn execute_command(app: &mut App, line: &str) {
    let mut parts = line.split_whitespace();
    let verb = parts.next().unwrap_or("");
    let arg = parts.next();

    let rows = dashboard_rows(service::snapshot());

    match verb {
        "start" => match arg.and_then(ServiceKind::from_name) {
            Some(k) => run_action(app, Action::Start(k)),
            None => app.status = "usage: start <bridge|zkteco|hrms>".to_string(),
        },
        "stop" => match arg {
            Some(name) => run_action(app, Action::Stop(name.to_string())),
            None => app.status = "usage: stop <instance>".to_string(),
        },
        "restart" => match arg.and_then(|name| row_by_name_or_kind(&rows, name)) {
            Some(row) => run_action(app, Action::Restart(row.name.clone(), row.kind)),
            None => app.status = "usage: restart <instance>".to_string(),
        },
        "sync" => run_action(app, Action::Sync(arg.map(str::to_string))),
        "logs" => match arg {
            Some("all") => {
                app.all_logs = true;
                app.show_logs = true;
                app.status = "showing logs: all running instances".to_string();
            }
            Some(name) => match row_index_by_name_or_kind(&rows, name) {
                Some(index) => {
                    app.selected = index;
                    app.all_logs = false;
                    app.show_logs = true;
                    app.status = format!("showing logs: {}", rows[index].name);
                }
                None => app.status = "usage: logs <instance>|all".to_string(),
            },
            None => app.status = "usage: logs <instance>|all".to_string(),
        },
        "select" => match arg.and_then(|name| row_index_by_name_or_kind(&rows, name)) {
            Some(index) => {
                app.selected = index;
                app.status = format!("selected {}", rows[index].name);
            }
            None => app.status = "usage: select <instance>|<kind>".to_string(),
        },
        "help" => {
            app.status =
                "commands: start <kind> · stop|restart <instance> · sync [code] · logs <name|all>"
                    .to_string()
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
    let visible = area.height.saturating_sub(2) as usize;
    let lines = if app.all_logs {
        service::tail_all_running(visible.max(1))
    } else {
        let name = rows
            .get(app.selected)
            .map(|r| r.name.as_str())
            .unwrap_or(ServiceKind::AtBridge.name());
        let log_path = paths::service_log_path(name);
        service::tail_lines(&log_path, LOG_TAIL)
    };
    let shown: Vec<Line> = lines
        .iter()
        .rev()
        .take(visible.max(1))
        .rev()
        .map(|l| Line::from(l.clone()))
        .collect();

    let title = if app.all_logs {
        " logs: all running instances ".to_string()
    } else {
        let row = rows.get(app.selected);
        format!(
            " logs: {} ({}) ",
            row.map(|r| r.name.as_str()).unwrap_or("bridge"),
            if row.map(|r| r.running).unwrap_or(false) {
                "running"
            } else {
                "stopped"
            }
        )
    };
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
    line1.extend(key("a", "all logs"));
    line1.extend(key("q", "quit"));

    let line2 = vec![
        Span::styled(":", Style::default().fg(Color::Yellow).bold()),
        Span::raw(" command  —  "),
        Span::styled(
            "start <kind> · stop|restart <instance> · sync [code] · logs <name|all> · select <name>",
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
