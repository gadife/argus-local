//! Terminal UI — full screens via ratatui (same engine as HTML).
use crate::insights::build_insights;
use crate::models::{InsightsPayload, LanguageLock, PeriodRollup, SessionRecord};
use crate::rollups::{format_tokens, rollup};
use crate::SessionStore;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::{Backend, CrosstermBackend};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, Tabs, Wrap};
use ratatui::{Frame, Terminal};
use std::io;
use std::path::Path;
use std::time::Duration;

const PAGES: &[&str] = &[
    "Today",
    "Activity",
    "Tools",
    "Insights",
    "Shipping",
    "Share",
    "Settings",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Today = 0,
    Activity = 1,
    Tools = 2,
    Insights = 3,
    Shipping = 4,
    Share = 5,
    Settings = 6,
}

impl Page {
    fn from_index(i: usize) -> Self {
        match i {
            0 => Page::Today,
            1 => Page::Activity,
            2 => Page::Tools,
            3 => Page::Insights,
            4 => Page::Shipping,
            5 => Page::Share,
            _ => Page::Settings,
        }
    }
    fn index(self) -> usize {
        self as usize
    }
    fn title(self) -> &'static str {
        PAGES[self.index()]
    }
}

struct App {
    page: Page,
    days: u32,
    today: PeriodRollup,
    insights: InsightsPayload,
    used_fixtures: bool,
    sessions: Vec<SessionRecord>,
    statuses: Vec<crate::models::AdapterStatus>,
    should_quit: bool,
}

impl App {
    fn new(store: SessionStore, days: u32) -> Self {
        let days = days.clamp(1, 90);
        let today = rollup(
            &store.sessions,
            days,
            store.statuses.clone(),
            store.used_fixtures,
        );
        let insights = build_insights(
            &store.sessions,
            days,
            store.statuses.clone(),
            store.used_fixtures,
        );
        Self {
            page: Page::Today,
            days,
            today,
            insights,
            used_fixtures: store.used_fixtures,
            sessions: store.sessions,
            statuses: store.statuses,
            should_quit: false,
        }
    }

    fn recompute(&mut self) {
        self.today = rollup(
            &self.sessions,
            self.days,
            self.statuses.clone(),
            self.used_fixtures,
        );
        self.insights = build_insights(
            &self.sessions,
            self.days,
            self.statuses.clone(),
            self.used_fixtures,
        );
    }

    fn next_page(&mut self) {
        self.page = Page::from_index((self.page.index() + 1) % PAGES.len());
    }

    fn prev_page(&mut self) {
        let i = self.page.index();
        self.page = Page::from_index(if i == 0 { PAGES.len() - 1 } else { i - 1 });
    }

    fn cycle_days(&mut self) {
        self.days = match self.days {
            1 => 7,
            7 => 30,
            _ => 1,
        };
        self.recompute();
    }

    fn set_days(&mut self, d: u32) {
        self.days = d;
        self.recompute();
    }
}

fn tok_label(n: i64, known: bool) -> String {
    if known {
        format_tokens(n)
    } else {
        "\u{2014}".into()
    }
}

fn usd_label(n: f64, known: bool) -> String {
    if known {
        format!("${:.0}", n)
    } else {
        "\u{2014}".into()
    }
}

fn green() -> Style {
    Style::default().fg(Color::Rgb(74, 222, 128))
}

fn muted() -> Style {
    Style::default().fg(Color::Rgb(156, 163, 175))
}

fn dim() -> Style {
    Style::default().fg(Color::Rgb(107, 114, 128))
}

fn warn() -> Style {
    Style::default().fg(Color::Rgb(245, 158, 11))
}

fn title_style() -> Style {
    Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}

/// Interactive full-screen TUI. q / Esc to quit.
pub fn run(store: SessionStore, days: u32) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut app = App::new(store, days);

    let res = loop_ui(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    res
}

fn loop_ui<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                    KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => app.next_page(),
                    KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => app.prev_page(),
                    KeyCode::Char('1') => app.page = Page::Today,
                    KeyCode::Char('2') => app.page = Page::Activity,
                    KeyCode::Char('3') => app.page = Page::Tools,
                    KeyCode::Char('4') => app.page = Page::Insights,
                    KeyCode::Char('5') => app.page = Page::Shipping,
                    KeyCode::Char('6') => app.page = Page::Share,
                    KeyCode::Char('7') => app.page = Page::Settings,
                    KeyCode::Char('d') | KeyCode::Char('[') | KeyCode::Char(']') => {
                        app.cycle_days()
                    }
                    KeyCode::Char('!') => app.set_days(1),
                    KeyCode::Char('@') => app.set_days(7),
                    KeyCode::Char('#') => app.set_days(30),
                    _ => {}
                }
            }
        }
        if app.should_quit {
            break;
        }
    }
    Ok(())
}

/// Non-interactive proof: render Today + Insights into UTF-8 screen dumps under `dir`.
pub fn write_proof_screens(store: SessionStore, days: u32, dir: &Path) -> Result<Vec<std::path::PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut app = App::new(store, days);
    let width = 100u16;
    let height = 36u16;
    let mut out_paths = Vec::new();

    for (page, name) in [(Page::Today, "tui-today"), (Page::Insights, "tui-insights")] {
        app.page = page;
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend)?;
        terminal.draw(|f| draw(f, &app))?;
        let buffer = terminal.backend().buffer().clone();
        let text = buffer_to_text(&buffer, width, height);
        let path = dir.join(format!("{name}.txt"));
        std::fs::write(&path, &text)?;
        out_paths.push(path);
    }
    Ok(out_paths)
}

fn buffer_to_text(buf: &ratatui::buffer::Buffer, width: u16, height: u16) -> String {
    let mut s = String::with_capacity((width as usize + 1) * height as usize);
    for y in 0..height {
        for x in 0..width {
            let cell = buf.cell((x, y)).unwrap();
            let ch = cell.symbol();
            if ch.is_empty() {
                s.push(' ');
            } else {
                s.push(ch.chars().next().unwrap_or(' '));
            }
        }
        s.push('\n');
    }
    s
}

fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(if app.used_fixtures { 2 } else { 0 }),
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area);

    draw_tabs(f, chunks[0], app);
    draw_header(f, chunks[1], app);
    if app.used_fixtures {
        draw_fixture_banner(f, chunks[2]);
    }
    draw_status(f, chunks[3], app);
    match app.page {
        Page::Today => draw_today(f, chunks[4], app),
        Page::Insights => draw_insights(f, chunks[4], app),
        other => draw_stub(f, chunks[4], other),
    }
    draw_footer(f, chunks[5], app);
}

fn draw_tabs(f: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = PAGES
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let label = format!(" {} ", p);
            if i == app.page.index() {
                Line::from(Span::styled(
                    label,
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Rgb(74, 222, 128))
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(Span::styled(label, muted()))
            }
        })
        .collect();
    let tabs = Tabs::new(titles)
        .select(app.page.index())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(" Argus Local ", green().add_modifier(Modifier::BOLD)))
                .border_style(Style::default().fg(Color::Rgb(42, 42, 42))),
        )
        .divider(Span::raw("|"));
    f.render_widget(tabs, area);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let lock = match app.page {
        Page::Today => app.today.language_lock.estimates_note.clone(),
        Page::Insights => app.insights.language_lock.observations_note.clone(),
        _ => LanguageLock::default().observations_note,
    };
    let days_hint = format!("period={}d (press d)", app.days);
    let line = Line::from(vec![
        Span::styled(
            format!("{} \u{2014} Last {} days", app.page.title(), app.days),
            title_style(),
        ),
        Span::raw("   "),
        Span::styled(lock, dim()),
        Span::raw("   "),
        Span::styled(days_hint, muted()),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

fn draw_fixture_banner(f: &mut Frame, area: Rect) {
    let p = Paragraph::new(Span::styled(
        " Demo fixtures \u{2014} not live adapter data. Label shown so this is never silent fake-as-real. ",
        warn(),
    ));
    f.render_widget(p, area);
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::styled("adapters: ", dim()));
    for (i, s) in app.today.adapter_status.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        let style = if s.partial || !s.ok { warn() } else { green() };
        spans.push(Span::styled(format!("* {}", s.name), style));
    }
    if app.today.adapter_status.is_empty() {
        spans.push(Span::styled("(none found on device)", dim()));
    }
    spans.push(Span::raw("   "));
    spans.push(Span::styled("Share to org: Off", muted()));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let foot = app.today.language_lock.footer.clone();
    let help = " Tab/h/l navigate · 1-7 pages · d period · q quit ";
    let line = Line::from(vec![
        Span::styled(foot, dim()),
        Span::raw("  |  "),
        Span::styled(help, muted()),
    ]);
    f.render_widget(
        Paragraph::new(line).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(Color::Rgb(42, 42, 42))),
        ),
        area,
    );
}

fn draw_today(f: &mut Frame, area: Rect, app: &App) {
    let d = &app.today;
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Min(6),
        ])
        .split(area);

    let kpis = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
            Constraint::Percentage(20),
        ])
        .split(outer[0]);

    let accept = d
        .tool_accept_pct
        .map(|p| format!("{:.0}%", p))
        .unwrap_or_else(|| "\u{2014}".into());

    render_kpi(f, kpis[0], "Sessions", &d.sessions.to_string(), "Completed");
    render_kpi(
        f,
        kpis[1],
        "Tokens",
        &tok_label(d.tokens, d.tokens_known),
        if d.tokens_known {
            "Total estimated"
        } else {
            "Unknown"
        },
    );
    render_kpi(
        f,
        kpis[2],
        "$ Est. spend",
        &usd_label(d.est_spend_usd, d.tokens_known),
        "estimate",
    );
    render_kpi(f, kpis[3], "Tool accept", &accept, "Accepted / proposed");
    render_kpi(
        f,
        kpis[4],
        "Models",
        &d.models_unique.to_string(),
        "Unique models",
    );

    let mut tool_lines: Vec<Line> = vec![Line::from(Span::styled(
        "By tool (found on device only)",
        muted().add_modifier(Modifier::BOLD),
    ))];
    if d.by_tool.is_empty() {
        tool_lines.push(Line::from(Span::styled("  (no tools in period)", dim())));
    } else {
        for t in &d.by_tool {
            let bar_w = ((t.share_pct / 100.0) * 20.0).round() as usize;
            let filled = "#".repeat(bar_w.min(20));
            let empty = "-".repeat(20usize.saturating_sub(bar_w));
            let bar = format!("{filled}{empty}");
            let warn_tag = if t.cost_incomplete {
                "  cost incomplete"
            } else {
                ""
            };
            tool_lines.push(Line::from(vec![
                Span::styled(format!("  {:<10}", t.tool), Style::default().fg(Color::White)),
                Span::styled(
                    format!(
                        "{:>6} tok  {:>3} sess  ",
                        tok_label(t.tokens, t.tokens_known),
                        t.sessions
                    ),
                    muted(),
                ),
                Span::styled(bar, green()),
                Span::styled(warn_tag, warn()),
            ]));
        }
    }
    f.render_widget(
        Paragraph::new(tool_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(42, 42, 42)))
                .title(" By tool "),
        ),
        outer[1],
    );

    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(outer[2]);

    let header = Row::new(vec!["Time", "Tool", "Model", "Tokens", "Est."]).style(dim());
    let rows: Vec<Row> = d
        .activity
        .iter()
        .map(|a| {
            Row::new(vec![
                Cell::from(a.time_range.clone()),
                Cell::from(a.tool.clone()),
                Cell::from(a.model.clone()),
                Cell::from(format!("{} tokens", tok_label(a.tokens, a.tokens_known))),
                Cell::from(usd_label(a.est_spend_usd, a.tokens_known)),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(13),
            Constraint::Length(10),
            Constraint::Min(16),
            Constraint::Length(14),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Rgb(42, 42, 42)))
            .title(" Activity "),
    );
    f.render_widget(table, mid[0]);

    let ship = &d.shipping;
    let stub = if ship.is_stub { " stub " } else { "" };
    let dash = "\u{2014}";
    let ship_lines = vec![
        Line::from(vec![
            Span::styled("Merged PRs", muted()),
            Span::raw("          "),
            Span::styled(dash, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Commits", muted()),
            Span::raw("            "),
            Span::styled(dash, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::styled("Files touched", muted()),
            Span::raw("      "),
            Span::styled(dash, Style::default().fg(Color::White)),
        ]),
        Line::from(""),
        Line::from(Span::styled(ship.note.clone(), dim())),
    ];
    f.render_widget(
        Paragraph::new(ship_lines)
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(42, 42, 42)))
                    .title(format!(" Shipping (opt-in GitHub){stub} ")),
            ),
        mid[1],
    );
}

fn render_kpi(f: &mut Frame, area: Rect, label: &str, value: &str, sub: &str) {
    let lines = vec![
        Line::from(Span::styled(label, muted())),
        Line::from(Span::styled(value, title_style())),
        Line::from(Span::styled(sub, dim())),
    ];
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(42, 42, 42))),
        ),
        area,
    );
}

fn draw_insights(f: &mut Frame, area: Rect, app: &App) {
    let d = &app.insights;
    let c = &d.context;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(area);

    let mut finding_lines: Vec<Line> = Vec::new();
    for (i, finding) in d.findings.iter().enumerate() {
        if i > 0 {
            finding_lines.push(Line::from(""));
        }
        finding_lines.push(Line::from(Span::styled(
            format!("> {}", finding.title),
            title_style().fg(Color::Rgb(74, 222, 128)),
        )));
        finding_lines.push(Line::from(Span::styled(
            finding.summary.clone(),
            Style::default().fg(Color::White),
        )));
        finding_lines.push(Line::from(Span::styled(finding.evidence.clone(), dim())));
    }
    f.render_widget(
        Paragraph::new(finding_lines)
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(42, 42, 42)))
                    .title(" Findings (observations, not a score) "),
            ),
        cols[0],
    );

    let accept = c
        .tool_accept_pct
        .map(|p| format!("{:.0}%", p))
        .unwrap_or_else(|| "\u{2014}".into());
    let mut ctx_lines = vec![
        Line::from(vec![
            Span::styled("Sessions", muted()),
            Span::raw("     "),
            Span::styled(c.sessions.to_string(), title_style()),
        ]),
        Line::from(vec![
            Span::styled("Tokens", muted()),
            Span::raw("       "),
            Span::styled(tok_label(c.tokens, c.tokens_known), title_style()),
        ]),
        Line::from(vec![
            Span::styled("Est. spend", muted()),
            Span::raw("   "),
            Span::styled(
                format!("{} estimate", usd_label(c.est_spend_usd, c.tokens_known)),
                title_style(),
            ),
        ]),
        Line::from(vec![
            Span::styled("Tool accept", muted()),
            Span::raw("  "),
            Span::styled(accept, title_style()),
        ]),
        Line::from(""),
        Line::from(Span::styled("By tool", muted().add_modifier(Modifier::BOLD))),
    ];
    for t in &c.by_tool {
        let bar_w = ((t.share_pct / 100.0) * 16.0).round() as usize;
        let filled = "#".repeat(bar_w.min(16));
        let empty = "-".repeat(16usize.saturating_sub(bar_w));
        let bar = format!("{filled}{empty}");
        ctx_lines.push(Line::from(vec![
            Span::styled(format!("{:<9}", t.tool), Style::default().fg(Color::White)),
            Span::styled(bar, green()),
            Span::styled(
                format!(" {}", tok_label(t.tokens, t.tokens_known)),
                muted(),
            ),
        ]));
    }
    ctx_lines.push(Line::from(""));
    ctx_lines.push(Line::from(Span::styled(
        "Not ranked vs peers \u{2014} not a productivity score",
        dim(),
    )));
    f.render_widget(
        Paragraph::new(ctx_lines)
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(42, 42, 42)))
                    .title(" Context "),
            ),
        cols[1],
    );
}

fn draw_stub(f: &mut Frame, area: Rect, page: Page) {
    let name = page.title();
    let body = match page {
        Page::Share => {
            "Share to org defaults Off (UI only in v1). No org Share bridge in this release.\n\naggregates only \u{2014} no raw prompts or code \u{2014} local-first"
        }
        Page::Shipping => {
            "Shipping is a stub / opt-in GitHub placeholder \u{2014} not observed shipping data.\n\ncorrelation with sessions \u{2014} not a productivity score."
        }
        _ => {
            "Navigation stub in v1. Today + Insights are live.\n\naggregates only \u{2014} no raw prompts or code \u{2014} local-first"
        }
    };
    let lines = vec![
        Line::from(Span::styled(format!("{} \u{2014} stub", name), title_style())),
        Line::from(""),
        Line::from(Span::styled(body, muted())),
        Line::from(""),
        Line::from(Span::styled(
            "Local only \u{2014} observations, not a score",
            dim(),
        )),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Rgb(42, 42, 42)))
                    .title(format!(" {} ", name)),
            ),
        area,
    );
}
