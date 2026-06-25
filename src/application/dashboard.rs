//! Live full-screen service dashboard (`fbsy dashboard`).
//!
//! A ratatui front-end over the same registry/process core that powers
//! `fbsy show`/`run`/`close`. It auto-refreshes, lets you start/stop/restart
//! the selected service, and tails its log file — without leaving the screen.

use std::{io::IsTerminal, time::Duration};

use anyhow::Result;
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style, Stylize},
    text::Line,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame,
};

use crate::{
    application::service::{self, ServiceStatus},
    services::ServiceKind,
    support::paths,
};

const TICK: Duration = Duration::from_millis(250);
const LOG_TAIL: usize = 200;

struct App {
    selected: usize,
    show_logs: bool,
    status: String,
    quit: bool,
}

impl App {
    fn new() -> Self {
        Self {
            selected: 0,
            show_logs: true,
            status: "↑/↓ select · s start · x stop · r restart · l logs · q quit".to_string(),
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

        if event::poll(TICK)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
                    KeyCode::Up | KeyCode::Char('k') => {
                        app.selected = app.selected.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if app.selected + 1 < count {
                            app.selected += 1;
                        }
                    }
                    KeyCode::Char('l') => app.show_logs = !app.show_logs,
                    KeyCode::Char('s') => {
                        let kind = app.selected_kind();
                        app.status = match service::default_start(kind) {
                            Ok(pid) => format!("started {} (pid {pid})", kind.name()),
                            Err(e) => format!("start {}: {e}", kind.name()),
                        };
                    }
                    KeyCode::Char('x') => {
                        let kind = app.selected_kind();
                        app.status = match service::stop_service(kind) {
                            Ok(true) => format!("stopped {}", kind.name()),
                            Ok(false) => format!("{} was not running", kind.name()),
                            Err(e) => format!("stop {}: {e}", kind.name()),
                        };
                    }
                    KeyCode::Char('r') => {
                        let kind = app.selected_kind();
                        let _ = service::stop_service(kind);
                        std::thread::sleep(Duration::from_millis(150));
                        app.status = match service::default_start(kind) {
                            Ok(pid) => format!("restarted {} (pid {pid})", kind.name()),
                            Err(e) => format!("restart {}: {e}", kind.name()),
                        };
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn draw(frame: &mut Frame, app: &App, rows: &[ServiceStatus]) {
    let log_constraint = if app.show_logs {
        Constraint::Min(6)
    } else {
        Constraint::Length(0)
    };
    let areas = Layout::vertical([
        Constraint::Length(3), // title
        Constraint::Length(7), // service table (3 services + header + borders)
        log_constraint,        // log pane
        Constraint::Length(1), // status line
    ])
    .split(frame.area());

    draw_title(frame, areas[0]);
    draw_table(frame, areas[1], app, rows);
    if app.show_logs {
        draw_logs(frame, areas[2], app, rows);
    }
    frame.render_widget(
        Line::from(app.status.clone()).style(Style::default().fg(Color::Yellow)),
        areas[3],
    );
}

fn draw_title(frame: &mut Frame, area: ratatui::layout::Rect) {
    let title = Paragraph::new(Line::from(vec![
        "fbsy".bold().cyan(),
        "  service dashboard".into(),
    ]))
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, area);
}

fn draw_table(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, rows: &[ServiceStatus]) {
    let header = Row::new(["SERVICE", "STATUS", "PID", "PORT", "UPTIME", "DESCRIPTION"])
        .style(Style::default().add_modifier(Modifier::BOLD));

    let body: Vec<Row> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let (status_text, status_color) = if r.running {
                ("running", Color::Green)
            } else {
                ("stopped", Color::DarkGray)
            };
            let pid = r.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into());
            let port = r.port.map(|p| p.to_string()).unwrap_or_else(|| "-".into());
            let uptime = r
                .uptime_secs
                .map(service::format_uptime_secs)
                .unwrap_or_else(|| "-".into());

            let mut row = Row::new(vec![
                Cell::from(r.kind.name()),
                Cell::from(status_text).style(Style::default().fg(status_color)),
                Cell::from(pid),
                Cell::from(port),
                Cell::from(uptime),
                Cell::from(r.kind.description()),
            ]);
            if i == app.selected {
                row = row.style(Style::default().add_modifier(Modifier::REVERSED));
            }
            row
        })
        .collect();

    let widths = [
        Constraint::Length(11),
        Constraint::Length(9),
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Length(8),
        Constraint::Min(20),
    ];
    let table = Table::new(body, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" services "));
    frame.render_widget(table, area);
}

fn draw_logs(frame: &mut Frame, area: ratatui::layout::Rect, app: &App, rows: &[ServiceStatus]) {
    let kind = app.selected_kind();
    let running = rows.get(app.selected).map(|r| r.running).unwrap_or(false);
    let log_path = paths::service_log_path(kind.name());

    let visible = area.height.saturating_sub(2) as usize;
    let lines = service::tail_lines(&log_path, LOG_TAIL.min(visible.max(1) * 4));
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
