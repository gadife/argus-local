//! Terminal UI — full screens via ratatui (same engine as HTML).
use crate::insights::build_insights;
use crate::adapters::{coach_dimensions, coaching_has_any, load_session_coaching, load_session_telemetry};
use crate::models::{InsightsPayload, LanguageLock, PeriodRollup, SessionRecord, SessionTelemetry};
use crate::rollups::{ShippingContext, format_tokens, rollup};
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
    shipping_events: Vec<crate::models::ShippingEvent>,
    github_configured: bool,
    github_auth_source: Option<String>,
    github_login: Option<String>,
    /// Selected row on Activity page (into today.activity).
    activity_idx: usize,
    /// When true, Activity shows the selected session detail pane focus.
    activity_detail: bool,
    /// Local prompts opt-in (settings.json prompts.enabled).
    prompts_enabled: bool,
    /// Expand truncated prompt/response in Activity detail.
    prompts_expand: bool,
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
            ShippingContext {
                configured: store.github_configured,
                events: &store.shipping_events,
                auth_source: store.github_auth_source.as_deref(),
                login: store.github_login.as_deref(),
            },
        );
        let insights = build_insights(
            &store.sessions,
            days,
            store.statuses.clone(),
            store.used_fixtures,
            ShippingContext {
                configured: store.github_configured,
                events: &store.shipping_events,
                auth_source: store.github_auth_source.as_deref(),
                login: store.github_login.as_deref(),
            },
        );
        Self {
            page: Page::Today,
            days,
            today,
            insights,
            used_fixtures: store.used_fixtures,
            sessions: store.sessions,
            statuses: store.statuses,
            shipping_events: store.shipping_events,
            github_configured: store.github_configured,
            github_auth_source: store.github_auth_source,
            github_login: store.github_login,
            activity_idx: 0,
            activity_detail: false,
            prompts_enabled: crate::settings::prompts_enabled(),
            prompts_expand: false,
            should_quit: false,
        }
    }

    fn toggle_prompts(&mut self) {
        let next = !self.prompts_enabled;
        if let Err(e) = crate::settings::set_prompts_enabled(next) {
            eprintln!("argus-local: prompts toggle failed: {e}");
            return;
        }
        self.prompts_enabled = crate::settings::prompts_enabled();
        self.prompts_expand = false;
    }


    fn activity_len(&self) -> usize {
        self.today.activity.len()
    }

    fn clamp_activity(&mut self) {
        let n = self.activity_len();
        if n == 0 {
            self.activity_idx = 0;
            self.activity_detail = false;
        } else if self.activity_idx >= n {
            self.activity_idx = n - 1;
        }
    }

    fn select_activity_delta(&mut self, delta: isize) {
        let n = self.activity_len();
        if n == 0 {
            return;
        }
        let cur = self.activity_idx as isize + delta;
        self.activity_idx = cur.clamp(0, (n as isize) - 1) as usize;
    }

    fn recompute(&mut self) {
        self.today = rollup(
            &self.sessions,
            self.days,
            self.statuses.clone(),
            self.used_fixtures,
            ShippingContext {
                configured: self.github_configured,
                events: &self.shipping_events,
                auth_source: self.github_auth_source.as_deref(),
                login: self.github_login.as_deref(),
            },
        );
        self.insights = build_insights(
            &self.sessions,
            self.days,
            self.statuses.clone(),
            self.used_fixtures,
            ShippingContext {
                configured: self.github_configured,
                events: &self.shipping_events,
                auth_source: self.github_auth_source.as_deref(),
                login: self.github_login.as_deref(),
            },
        );
            self.clamp_activity();
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

fn info() -> Style {
    Style::default().fg(Color::Rgb(96, 165, 250))
}

fn title_style() -> Style {
    Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD)
}

fn chrome() -> Style {
    // Muted panel borders — k9s/lazygit sparse chrome (one accent elsewhere).
    Style::default().fg(Color::Rgb(55, 65, 81))
}

fn accent() -> Style {
    green()
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(chrome())
        .title(Span::styled(format!(" {title} "), muted()))
}

/// btop-ish share meter using Unicode block elements (no Nerd Fonts).
fn meter(pct: f64, width: usize) -> String {
    const BLOCKS: &[char] = &[' ', '\u{2581}', '\u{2582}', '\u{2583}', '\u{2584}', '\u{2585}', '\u{2586}', '\u{2587}', '\u{2588}'];
    if width == 0 {
        return String::new();
    }
    let pct = pct.clamp(0.0, 100.0);
    let full = ((pct / 100.0) * width as f64).floor() as usize;
    let rem = ((pct / 100.0) * width as f64) - full as f64;
    let partial = (rem * (BLOCKS.len() - 1) as f64).round() as usize;
    let mut out = String::with_capacity(width);
    for i in 0..width {
        if i < full {
            out.push('\u{2588}');
        } else if i == full && partial > 0 {
            out.push(BLOCKS[partial.min(BLOCKS.len() - 1)]);
        } else {
            out.push('\u{2591}');
        }
    }
    out
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
                    KeyCode::Char('q') => app.should_quit = true,
                    KeyCode::Esc => {
                        if app.page == Page::Activity && app.activity_detail {
                            app.activity_detail = false;
                        } else {
                            app.should_quit = true;
                        }
                    }
                    KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                        app.activity_detail = false;
                        app.next_page();
                    }
                    KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                        app.activity_detail = false;
                        app.prev_page();
                    }
                    KeyCode::Char('1') => {
                        app.activity_detail = false;
                        app.page = Page::Today;
                    }
                    KeyCode::Char('2') => app.page = Page::Activity,
                    KeyCode::Char('3') => {
                        app.activity_detail = false;
                        app.page = Page::Tools;
                    }
                    KeyCode::Char('4') => {
                        app.activity_detail = false;
                        app.page = Page::Insights;
                    }
                    KeyCode::Char('5') => {
                        app.activity_detail = false;
                        app.page = Page::Shipping;
                    }
                    KeyCode::Char('6') => {
                        app.activity_detail = false;
                        app.page = Page::Share;
                    }
                    KeyCode::Char('7') => {
                        app.activity_detail = false;
                        app.page = Page::Settings;
                    }
                    KeyCode::Char('d') | KeyCode::Char('[') | KeyCode::Char(']') => {
                        app.cycle_days();
                        app.clamp_activity();
                    }
                    KeyCode::Char('!') => {
                        app.set_days(1);
                        app.clamp_activity();
                    }
                    KeyCode::Char('@') => {
                        app.set_days(7);
                        app.clamp_activity();
                    }
                    KeyCode::Char('#') => {
                        app.set_days(30);
                        app.clamp_activity();
                    }
                    KeyCode::Up | KeyCode::Char('k') if app.page == Page::Activity => {
                        app.select_activity_delta(-1);
                    }
                    KeyCode::Down | KeyCode::Char('j') if app.page == Page::Activity => {
                        app.select_activity_delta(1);
                    }
                    KeyCode::Enter if app.page == Page::Activity => {
                        if app.activity_len() > 0 {
                            app.activity_detail = true;
                        }
                    }
                    KeyCode::Enter if app.page == Page::Today => {
                        // Optional: jump to Activity with first row selected
                        app.page = Page::Activity;
                        app.activity_detail = app.activity_len() > 0;
                    }
                    KeyCode::Char('p') if app.page == Page::Settings => {
                        app.toggle_prompts();
                    }
                    KeyCode::Char('e')
                        if app.page == Page::Activity && app.prompts_enabled =>
                    {
                        app.prompts_expand = !app.prompts_expand;
                    }
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
    let height = 90u16;
    let mut out_paths = Vec::new();

    for (page, name) in [(Page::Today, "tui-today"), (Page::Activity, "tui-activity"), (Page::Insights, "tui-insights"), (Page::Shipping, "tui-shipping"), (Page::Settings, "tui-settings")] {
        app.page = page;
        if page == Page::Activity {
            // Prefer richest ARG-40 coaching session; else ARG-37 fixture / live Grok; else incomplete.
            let idx = {
                let scored: Vec<(usize, i64)> = app
                    .today
                    .activity
                    .iter()
                    .enumerate()
                    .filter_map(|(i, a)| {
                        let c = load_session_coaching(&a.id).ok()?;
                        if !coaching_has_any(&c) {
                            return None;
                        }
                        let mut score = 0i64;
                        if c.tool_failure_count.is_some() {
                            score += 4;
                        }
                        if c.identical_tool_streak.is_some() {
                            score += 4;
                        }
                        if c.explore_count.is_some() {
                            score += 1;
                        }
                        if c.checks_after_edits.is_some() {
                            score += 2;
                        }
                        if c.subagent_spawn_count.is_some() || c.subagent_dir_count.is_some() {
                            score += 4;
                        }
                        Some((i, score))
                    })
                    .collect();
                scored
                    .into_iter()
                    .max_by_key(|(_, s)| *s)
                    .map(|(i, _)| i)
                    .or_else(|| {
                        app.today
                            .activity
                            .iter()
                            .position(|a| a.id == crate::adapters::FIXTURE_PROMPTS_ID)
                    })
                    .or_else(|| {
                        app.today.activity.iter().position(|a| {
                            a.id.contains("01a0881e-e77a-78e3-a262-6cdb294be825")
                        })
                    })
                    .or_else(|| {
                        app.today.activity.iter().position(|a| {
                            a.id.starts_with("grok:")
                                && load_session_telemetry(&a.id)
                                    .ok()
                                    .and_then(|t| t.tool_call_count)
                                    .unwrap_or(0)
                                    > 0
                        })
                    })
                    .or_else(|| app.today.activity.iter().position(|a| !a.cost_complete))
            };
            if let Some(i) = idx {
                app.activity_idx = i;
                app.activity_detail = true;
            }
        }
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
        Page::Activity => draw_activity(f, chunks[4], app),
        Page::Insights => draw_insights(f, chunks[4], app),
        Page::Shipping => draw_shipping(f, chunks[4], app),
        Page::Settings => draw_settings(f, chunks[4], app),
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
                Line::from(Span::styled(label, dim()))
            }
        })
        .collect();
    let tabs = Tabs::new(titles)
        .select(app.page.index())
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(chrome())
                .title(Span::styled(" Argus Local ", accent().add_modifier(Modifier::BOLD))),
        )
        .divider(Span::styled("\u{2502}", chrome()));
    f.render_widget(tabs, area);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let lock = match app.page {
        Page::Today => app.today.language_lock.estimates_note.clone(),
        Page::Insights => app.insights.language_lock.observations_note.clone(),
        Page::Shipping => app.today.language_lock.shipping_note.clone(),
        Page::Settings => "opt-in GitHub + local prompts - never invent / never upload".into(),
        Page::Activity => LanguageLock::default().observations_note,
        _ => LanguageLock::default().observations_note,
    };
    let period_chip = format!(" {}d ", app.days);
    let mut spans = vec![
        Span::styled(format!("{} ", app.page.title()), title_style()),
        Span::styled(
            period_chip,
            Style::default()
                .fg(Color::Rgb(209, 213, 219))
                .bg(Color::Rgb(31, 41, 55)),
        ),
        Span::raw("  "),
    ];
    if app.prompts_enabled {
        spans.push(Span::styled(
            " Prompts on (local) ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(74, 222, 128))
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw("  "));
    }
    spans.push(Span::styled(lock, dim()));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
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
    spans.push(Span::styled(" adapters ", dim()));
    if app.today.adapter_status.is_empty() {
        spans.push(Span::styled(
            "[ none ]",
            Style::default()
                .fg(Color::Rgb(156, 163, 175))
                .bg(Color::Rgb(31, 41, 55)),
        ));
    } else {
        for (i, s) in app.today.adapter_status.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(" "));
            }
            let (tag, fg, bg) = if !s.ok {
                ("missing", Color::Rgb(252, 165, 165), Color::Rgb(69, 26, 26))
            } else if s.partial {
                ("partial", Color::Rgb(253, 230, 138), Color::Rgb(66, 32, 6))
            } else {
                ("ok", Color::Rgb(167, 243, 208), Color::Rgb(6, 46, 32))
            };
            let chip = format!(" {} \u{00b7} {} ", s.name, tag);
            spans.push(Span::styled(chip, Style::default().fg(fg).bg(bg)));
        }
    }
    spans.push(Span::raw("   "));
    spans.push(Span::styled(
        " Share \u{00b7} Off ",
        Style::default()
            .fg(Color::Rgb(156, 163, 175))
            .bg(Color::Rgb(31, 41, 55)),
    ));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn keymap_for(page: Page, prompts_on: bool) -> &'static str {
    match page {
        Page::Today => "tab/hl pages \u{00b7} d period \u{00b7} 1 today \u{00b7} 4 insights \u{00b7} q quit",
        Page::Insights => "tab/hl pages \u{00b7} d period \u{00b7} 1 today \u{00b7} 4 insights \u{00b7} q quit",
        Page::Activity if prompts_on => {
            "j/k select \u{00b7} Enter detail \u{00b7} e expand \u{00b7} Esc back \u{00b7} q quit"
        }
        Page::Activity => "j/k select \u{00b7} Enter detail \u{00b7} Esc back \u{00b7} tab/hl \u{00b7} q quit",
        Page::Tools => "stub view \u{00b7} tab/hl pages \u{00b7} 1 today \u{00b7} 4 insights \u{00b7} q quit",
        Page::Shipping => "shipping \u{00b7} tab/hl \u{00b7} 1 today \u{00b7} q quit",
        Page::Share => "share off (v1) \u{00b7} tab/hl \u{00b7} 1 today \u{00b7} q quit",
        Page::Settings => "p toggle prompts \u{00b7} tab/hl \u{00b7} 1 today \u{00b7} q quit",
    }
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let foot = if app.page == Page::Activity && app.prompts_enabled {
        "local prompts on \u{2014} never uploaded \u{2014} Share Off".to_string()
    } else {
        app.today.language_lock.footer.clone()
    };
    let help = keymap_for(app.page, app.prompts_enabled);
    let line = Line::from(vec![
        Span::styled(foot, dim()),
        Span::styled("  \u{2502}  ", chrome()),
        Span::styled(help, muted()),
    ]);
    f.render_widget(
        Paragraph::new(line).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(chrome()),
        ),
        area,
    );
}

fn draw_today(f: &mut Frame, area: Rect, app: &App) {
    let d = &app.today;
    // Soft hold: height must fit every tool in the rollup (borders + header + rows).
    // Fixed Length(6) previously clipped Grok when 4 tools were present.
    let tool_rows = d.by_tool.len().max(1);
    let by_tool_h = (2u16 + 1 + tool_rows as u16).max(4);
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(by_tool_h),
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

    render_kpi(f, kpis[0], &d.sessions.to_string(), "sessions", "completed");
    render_kpi(
        f,
        kpis[1],
        &tok_label(d.tokens, d.tokens_known),
        "tokens",
        if d.tokens_known { "estimated" } else { "unknown" },
    );
    render_kpi(
        f,
        kpis[2],
        &usd_label(d.est_spend_usd, d.tokens_known),
        "est. spend",
        "estimate \u{2260} invoice",
    );
    render_kpi(f, kpis[3], &accept, "tool accept", "accepted / proposed");
    render_kpi(f, kpis[4], &d.models_unique.to_string(), "models", "unique");

    let mut tool_lines: Vec<Line> = vec![Line::from(Span::styled(
        "found on device only",
        dim(),
    ))];
    if d.by_tool.is_empty() {
        tool_lines.push(Line::from(Span::styled("  (no tools in period)", dim())));
    } else {
        for t in &d.by_tool {
            let bar = meter(t.share_pct, 18);
            let warn_tag = if t.cost_incomplete {
                "  cost incomplete"
            } else {
                ""
            };
            tool_lines.push(Line::from(vec![
                Span::styled(format!(" {:<11}", t.tool), Style::default().fg(Color::White)),
                Span::styled(
                    format!(
                        "{:>6}  {:>3}s  ",
                        tok_label(t.tokens, t.tokens_known),
                        t.sessions
                    ),
                    muted(),
                ),
                Span::styled(bar, accent()),
                Span::styled(format!(" {:>4.0}%", t.share_pct), dim()),
                Span::styled(warn_tag, warn()),
            ]));
        }
    }
    f.render_widget(Paragraph::new(tool_lines).block(panel("by tool")), outer[1]);

    let mid = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(65), Constraint::Percentage(35)])
        .split(outer[2]);

    let header = Row::new(vec!["time", "tool", "model", "tokens", "est."]).style(dim());
    let rows: Vec<Row> = d
        .activity
        .iter()
        .map(|a| {
            Row::new(vec![
                Cell::from(a.time_range.clone()).style(muted()),
                Cell::from(a.tool.clone()),
                Cell::from(a.model.clone()).style(muted()),
                Cell::from(tok_label(a.tokens, a.tokens_known)),
                Cell::from(usd_label(a.est_spend_usd, a.tokens_known && a.cost_complete)).style(dim()),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(13),
            Constraint::Length(10),
            Constraint::Min(16),
            Constraint::Length(10),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(panel("activity"));
    f.render_widget(table, mid[0]);

    let ship = &d.shipping;
    let stub = if ship.is_stub { " \u{00b7} unconfigured" } else { "" };
    let dash = "\u{2014}";
    let fmt_ship = |v: Option<i64>| -> String { if ship.is_stub { dash.to_string() } else { v.map(|n| n.to_string()).unwrap_or_else(|| dash.to_string()) } };
    let ship_style = if ship.is_stub { muted() } else { title_style() };
    let ship_lines = vec![
        Line::from(vec![
            Span::styled("merged prs", dim()),
            Span::raw("      "),
            Span::styled(fmt_ship(ship.merged_prs), ship_style),
        ]),
        Line::from(vec![
            Span::styled("commits", dim()),
            Span::raw("        "),
            Span::styled(fmt_ship(ship.commits), ship_style),
        ]),
        Line::from(vec![
            Span::styled("files touched", dim()),
            Span::raw("  "),
            Span::styled(fmt_ship(ship.files_touched), ship_style),
        ]),
        Line::from(""),
        Line::from(Span::styled(ship.note.clone(), dim())),
    ];
    f.render_widget(
        Paragraph::new(ship_lines)
            .wrap(Wrap { trim: true })
            .block(panel(&format!("shipping{stub}"))),
        mid[1],
    );
}

fn render_kpi(f: &mut Frame, area: Rect, value: &str, label: &str, sub: &str) {
    let lines = vec![
        Line::from(Span::styled(value, title_style())),
        Line::from(Span::styled(label, muted())),
        Line::from(Span::styled(sub, dim())),
    ];
    f.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::RIGHT)
                .border_style(chrome()),
        ),
        area,
    );
}

fn finding_tint(title: &str) -> Style {
    let t = title.to_lowercase();
    if t.contains("incomplete") || t.contains("stub") || t.contains("unknown") {
        warn().add_modifier(Modifier::BOLD)
    } else if t.contains("failure")
        || t.contains("identical")
        || t.contains("few checks")
    {
        warn().add_modifier(Modifier::BOLD)
    } else if t.contains("accept")
        || t.contains("took more")
        || t.contains("dominated")
        || t.contains("explore")
        || t.contains("subagent")
        || t.contains("checks after")
    {
        info().add_modifier(Modifier::BOLD)
    } else {
        accent().add_modifier(Modifier::BOLD)
    }
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
        let tint = finding_tint(&finding.title);
        let marker = if finding.title.to_lowercase().contains("incomplete")
            || finding.title.to_lowercase().contains("stub")
        {
            "!"
        } else {
            "i"
        };
        finding_lines.push(Line::from(vec![
            Span::styled(format!(" {marker} "), tint),
            Span::styled(finding.title.clone(), tint),
        ]));
        finding_lines.push(Line::from(Span::styled(
            format!("   {}", finding.summary),
            Style::default().fg(Color::White),
        )));
        finding_lines.push(Line::from(Span::styled(
            format!("   {}", finding.evidence),
            dim(),
        )));
    }
    f.render_widget(
        Paragraph::new(finding_lines)
            .wrap(Wrap { trim: true })
            .block(panel("findings \u{00b7} observations, not a score")),
        cols[0],
    );

    let accept = c
        .tool_accept_pct
        .map(|p| format!("{:.0}%", p))
        .unwrap_or_else(|| "\u{2014}".into());
    let mut ctx_lines = vec![
        Line::from(vec![
            Span::styled(c.sessions.to_string(), title_style()),
            Span::styled("  sessions", dim()),
        ]),
        Line::from(vec![
            Span::styled(tok_label(c.tokens, c.tokens_known), title_style()),
            Span::styled("  tokens", dim()),
        ]),
        Line::from(vec![
            Span::styled(usd_label(c.est_spend_usd, c.tokens_known), title_style()),
            Span::styled("  est. spend", dim()),
        ]),
        Line::from(vec![
            Span::styled(accept, title_style()),
            Span::styled("  tool accept", dim()),
        ]),
        Line::from(""),
        Line::from(Span::styled("by tool", muted())),
    ];
    for t in &c.by_tool {
        let bar = meter(t.share_pct, 14);
        ctx_lines.push(Line::from(vec![
            Span::styled(format!("{:<10}", t.tool), Style::default().fg(Color::White)),
            Span::styled(bar, accent()),
            Span::styled(
                format!(" {}", tok_label(t.tokens, t.tokens_known)),
                muted(),
            ),
        ]));
    }
    ctx_lines.push(Line::from(""));
    ctx_lines.push(Line::from(Span::styled(
        "not ranked vs peers \u{2014} not a productivity score",
        dim(),
    )));
    f.render_widget(
        Paragraph::new(ctx_lines)
            .wrap(Wrap { trim: true })
            .block(panel("context")),
        cols[1],
    );
}

fn draw_shipping(f: &mut Frame, area: Rect, app: &App) {
    let ship = &app.today.shipping;
    let dash = "\u{2014}";
    let fmt = |v: Option<i64>| -> String {
        if ship.is_stub { dash.to_string() } else { v.map(|n| n.to_string()).unwrap_or_else(|| dash.to_string()) }
    };
    let title = if ship.is_stub { "Shipping \u{2014} unconfigured" } else { "Shipping \u{2014} GitHub" };
    let mut lines = vec![
        Line::from(Span::styled(title, title_style())),
        Line::from(""),
        Line::from(vec![Span::styled("merged PRs      ", dim()), Span::styled(fmt(ship.merged_prs), if ship.is_stub { muted() } else { accent().add_modifier(Modifier::BOLD) })]),
        Line::from(vec![Span::styled("commits         ", dim()), Span::styled(fmt(ship.commits), if ship.is_stub { muted() } else { accent().add_modifier(Modifier::BOLD) })]),
        Line::from(vec![Span::styled("files touched   ", dim()), Span::styled(fmt(ship.files_touched), if ship.is_stub { muted() } else { accent().add_modifier(Modifier::BOLD) })]),
        Line::from(""),
        Line::from(Span::styled(ship.note.clone(), muted())),
        Line::from(""),
        Line::from(Span::styled("correlation with sessions \u{2014} not a productivity score", dim())),
        Line::from(Span::styled("estimates \u{2260} invoice \u{00b7} observations, not a score \u{00b7} Share Off default", dim())),
    ];
    if let Some(login) = &ship.login {
        lines.insert(2, Line::from(Span::styled(format!("user: {login} via {}", ship.auth_source.as_deref().unwrap_or("?")), muted())));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }).block(panel("shipping")), area);
}

fn draw_settings(f: &mut Frame, area: Rect, app: &App) {
    let path = crate::adapters::settings_path_display();
    let opted = crate::adapters::is_opted_in();
    let configured = app.github_configured;
    let prompts = if app.prompts_enabled {
        "ON (local)"
    } else {
        "OFF (default)"
    };
    let body = format!(
        "Local prompts (opt-in)\n\nstatus: {prompts}\npress p to toggle\n\nWhen ON: Activity session detail can show on-device prompt/response text.\nLocal only \u{2014} never uploaded. Share stays Off.\nWhen OFF: aggregates only \u{2014} no raw prompts.\nPersisted as prompts.enabled in:\n  {path}\n\nGitHub shipping (opt-in)\n\nconfigured now: {configured}\nopted in: {opted}\n\nHow to configure (no secrets in repo):\n  1. Set ARGUS_GITHUB_TOKEN to a personal PAT, or\n  2. Set ARGUS_GITHUB_ENABLED=1 and use authenticated gh, or\n  3. Write {{\"github\":{{\"enabled\":true}}}} to the same settings file.\n\nPrefer gh when authenticated. Never invents counts when unconfigured.\nShare to org defaults Off."
    );
    let lines = vec![
        Line::from(Span::styled("Settings", title_style())),
        Line::from(""),
        Line::from(Span::styled(body, muted())),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(panel("settings")),
        area,
    );
}


fn draw_activity(f: &mut Frame, area: Rect, app: &App) {
    let list = &app.today.activity;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(area);

    let header = Row::new(vec!["time", "tool", "model", "tokens", "dur"]).style(dim());
    let rows: Vec<Row> = list
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let style = if i == app.activity_idx {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(74, 222, 128))
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(a.time_range.clone()),
                Cell::from(a.tool.clone()),
                Cell::from(if a.model.is_empty() {
                    "\u{2014}".into()
                } else {
                    a.model.clone()
                }),
                Cell::from(tok_label(a.tokens, a.tokens_known)),
                Cell::from(if a.duration.is_empty() {
                    "\u{2014}".into()
                } else {
                    a.duration.clone()
                }),
            ])
            .style(style)
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(13),
            Constraint::Length(10),
            Constraint::Min(12),
            Constraint::Length(8),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(panel(if app.prompts_enabled { "sessions \u{00b7} prompts opt-in" } else { "sessions \u{00b7} aggregates only" }));
    f.render_widget(table, cols[0]);

    let selected = list.get(app.activity_idx);
    let mut lines: Vec<Line> = Vec::new();
    match selected {
        None => {
            lines.push(Line::from(Span::styled(
                "No sessions in period",
                muted(),
            )));
        }
        Some(a) => {
            let title = if app.activity_detail {
                "session detail"
            } else {
                "session detail \u{00b7} Enter"
            };
            lines.push(Line::from(Span::styled(title, dim())));
            lines.push(Line::from(""));
            let push_kv = |lines: &mut Vec<Line>, k: &str, v: String| {
                lines.push(Line::from(vec![
                    Span::styled(format!("{:<16}", k), dim()),
                    Span::styled(v, Style::default().fg(Color::White)),
                ]));
            };
            let push_sec = |lines: &mut Vec<Line>, name: &str| {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(name.to_string(), title_style())));
            };

            let tel = load_session_telemetry(&a.id).unwrap_or_else(|_| SessionTelemetry {
                id: a.id.clone(),
                tool: a.tool.clone(),
                ..Default::default()
            });

            push_kv(&mut lines, "id", a.id.clone());
            if let Some(ref t) = tel.generated_title {
                push_kv(&mut lines, "title", t.clone());
            }
            if let Some(ref k) = tel.session_kind {
                push_kv(&mut lines, "kind", k.clone());
            }
            if let Some(ref cwd) = tel.cwd {
                push_kv(&mut lines, "cwd", cwd.clone());
            }
            push_kv(&mut lines, "tool", a.tool.clone());
            let model = tel
                .model
                .clone()
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| {
                    if a.model.is_empty() {
                        "\u{2014}".into()
                    } else {
                        a.model.clone()
                    }
                });
            push_kv(&mut lines, "model", model);

            // Context
            if tel.context_tokens_used.is_some() || tel.context_window_tokens.is_some() {
                push_sec(&mut lines, "Context");
                match (tel.context_tokens_used, tel.context_window_tokens, tel.context_pct) {
                    (Some(u), Some(w), Some(p)) => {
                        push_kv(
                            &mut lines,
                            "used/window",
                            format!("{} / {} ({:.1}%)", format_tokens(u), format_tokens(w), p),
                        );
                    }
                    (Some(u), Some(w), None) => {
                        push_kv(
                            &mut lines,
                            "used/window",
                            format!("{} / {}", format_tokens(u), format_tokens(w)),
                        );
                    }
                    (Some(u), None, _) => {
                        push_kv(&mut lines, "context used", format_tokens(u));
                    }
                    _ => {}
                }
                if let Some(ref n) = tel.context_note {
                    lines.push(Line::from(Span::styled(n.clone(), dim())));
                }
            } else if !a.tokens_known {
                push_kv(&mut lines, "tokens", "\u{2014}".into());
            } else if !a.cost_complete {
                push_kv(
                    &mut lines,
                    "tokens/context",
                    format!("{} context (not billable in/out)", tok_label(a.tokens, true)),
                );
            }

            // Billable I/O
            let has_io = tel.input_tokens.is_some()
                || tel.output_tokens.is_some()
                || tel.total_tokens.is_some()
                || tel.reasoning_tokens.is_some();
            if has_io {
                push_sec(&mut lines, "Billable I/O");
                if let Some(v) = tel.input_tokens {
                    push_kv(&mut lines, "input", format_tokens(v));
                }
                if let Some(v) = tel.output_tokens {
                    push_kv(&mut lines, "output", format_tokens(v));
                }
                if let Some(v) = tel.total_tokens {
                    push_kv(&mut lines, "total", format_tokens(v));
                }
                if let Some(v) = tel.reasoning_tokens {
                    push_kv(&mut lines, "reasoning", format_tokens(v));
                }
                if let Some(v) = tel.model_calls {
                    push_kv(&mut lines, "model calls", v.to_string());
                }
                if let Some(v) = tel.api_duration_ms {
                    push_kv(&mut lines, "api ms", v.to_string());
                }
                if let Some(c) = tel.cost_usd_estimate {
                    push_kv(
                        &mut lines,
                        "est. spend",
                        format!(
                            "${:.4} {}",
                            c,
                            tel.cost_note.as_deref().unwrap_or("estimate \u{2260} invoice")
                        ),
                    );
                }
            } else if a.cost_complete && a.tokens_known {
                push_sec(&mut lines, "Billable I/O");
                push_kv(
                    &mut lines,
                    "tokens",
                    format!(
                        "{} (in {} / out {})",
                        tok_label(a.tokens, true),
                        tok_label(a.input_tokens, true),
                        tok_label(a.output_tokens, true)
                    ),
                );
            }

            // Cache
            if tel.cached_read_tokens.is_some() || tel.cache_creation_tokens.is_some() {
                push_sec(&mut lines, "Cache");
                if let Some(v) = tel.cached_read_tokens {
                    push_kv(&mut lines, "cached read", format_tokens(v));
                }
                if let Some(v) = tel.cache_creation_tokens {
                    push_kv(&mut lines, "cache create", format_tokens(v));
                }
            }

            // Signals
            let has_sig = tel.turn_count.is_some()
                || tel.tool_call_count.is_some()
                || tel.avg_time_to_first_token_ms.is_some()
                || tel.session_duration_seconds.is_some();
            if has_sig {
                push_sec(&mut lines, "Signals");
                if let Some(v) = tel.turn_count {
                    push_kv(&mut lines, "turns", v.to_string());
                }
                if let Some(v) = tel.tool_call_count {
                    push_kv(&mut lines, "tool calls", v.to_string());
                }
                if let Some(ref tools) = tel.tools_used {
                    if !tools.is_empty() {
                        push_kv(&mut lines, "tools used", tools.join(", "));
                    }
                }
                if let Some(v) = tel.avg_time_to_first_token_ms {
                    push_kv(&mut lines, "avg TTFT", format!("{:.0} ms", v));
                }
                if let Some(v) = tel.avg_response_time_ms {
                    push_kv(&mut lines, "avg resp", format!("{:.0} ms", v));
                }
                if let Some(v) = tel.session_duration_seconds {
                    push_kv(&mut lines, "duration s", v.to_string());
                }
            } else {
                push_kv(&mut lines, "tools prop.", a.tools_proposed.to_string());
                push_kv(&mut lines, "tools accept", a.tools_accepted.to_string());
            }

            // Coaching observations (ARG-40) — hide when absent
            let coach = load_session_coaching(&a.id).unwrap_or_default();
            if coaching_has_any(&coach) {
                push_sec(&mut lines, "Coaching");
                if let Some(n) = coach.tool_failure_count {
                    let denom = coach
                        .tool_call_count
                        .map(|c| format!(" / {c} calls"))
                        .unwrap_or_default();
                    push_kv(&mut lines, "failures", format!("{n}{denom}"));
                }
                if let (Some(ref name), Some(streak)) =
                    (&coach.identical_tool_name, coach.identical_tool_streak)
                {
                    push_kv(&mut lines, "identical", format!("`{name}` x{streak}"));
                }
                if coach.explore_count.is_some() || coach.act_count.is_some() {
                    push_kv(
                        &mut lines,
                        "explore/act",
                        format!(
                            "{} explore / {} act",
                            coach.explore_count.unwrap_or(0),
                            coach.act_count.unwrap_or(0)
                        ),
                    );
                }
                if let (Some(c), Some(e)) = (coach.checks_after_edits, coach.edits_count) {
                    let w = coach.checks_window.unwrap_or(5);
                    push_kv(&mut lines, "checks", format!("{c} of {e} within {w}"));
                }
                match (coach.subagent_spawn_count, coach.subagent_dir_count) {
                    (Some(s), Some(d)) => {
                        push_kv(&mut lines, "subagents", format!("{s} spawned / {d} dirs"))
                    }
                    (Some(s), None) => push_kv(&mut lines, "subagents", format!("{s} spawned")),
                    (None, Some(d)) => push_kv(&mut lines, "subagents", format!("{d} dirs")),
                    (None, None) => {}
                }
                lines.push(Line::from(Span::styled(
                    "observations, not a score",
                    dim(),
                )));
            }

            // Coach dimensions (ARG-42) — hide when empty
            let dims = coach_dimensions(&coach);
            if !dims.is_empty() {
                push_sec(&mut lines, "Coach");
                for d in &dims {
                    lines.push(Line::from(vec![
                        Span::styled(format!("{:<18}", d.title), Style::default().fg(Color::White)),
                        Span::styled(
                            format!("[{}]", d.label.as_str()),
                            match d.label {
                                crate::models::CoachLabel::Observed => Style::default().fg(Color::Green),
                                crate::models::CoachLabel::Watch => Style::default().fg(Color::Yellow),
                                crate::models::CoachLabel::Thin => Style::default().fg(Color::Red),
                            },
                        ),
                    ]));
                    lines.push(Line::from(Span::styled(format!("  {}", d.tip), dim())));
                }
                lines.push(Line::from(Span::styled(
                    "coach, don't rank - no score",
                    dim(),
                )));
            }

            if !a.adapter_note.is_empty() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(a.adapter_note.clone(), warn())));
            }
            lines.push(Line::from(""));
            if app.prompts_enabled {
                let texts = crate::adapters::load_session_texts(&a.id).unwrap_or_default();
                let trunc = |s: &str, expand: bool| -> String {
                    const N: usize = 280;
                    if expand || s.chars().count() <= N {
                        s.to_string()
                    } else {
                        let t: String = s.chars().take(N).collect();
                        format!("{t}\u{2026} [e expand]")
                    }
                };
                match (&texts.prompt_text, &texts.response_text) {
                    (None, None) => {
                        lines.push(Line::from(Span::styled(
                            "prompts missing \u{2014} no on-disk prompt/response text",
                            dim(),
                        )));
                    }
                    _ => {
                        if let Some(p) = &texts.prompt_text {
                            lines.push(Line::from(Span::styled("prompt", dim())));
                            lines.push(Line::from(Span::styled(
                                trunc(p, app.prompts_expand),
                                Style::default().fg(Color::White),
                            )));
                            lines.push(Line::from(""));
                        }
                        if let Some(r) = &texts.response_text {
                            lines.push(Line::from(Span::styled("response", dim())));
                            lines.push(Line::from(Span::styled(
                                trunc(r, app.prompts_expand),
                                Style::default().fg(Color::White),
                            )));
                        }
                        lines.push(Line::from(""));
                        lines.push(Line::from(Span::styled(
                            "local only \u{2014} never uploaded \u{2014} Share Off",
                            dim(),
                        )));
                    }
                }
            } else {
                lines.push(Line::from(Span::styled(
                    "aggregates only \u{2014} no raw prompts",
                    dim(),
                )));
            }
        }
    }
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(panel("detail")),
        cols[1],
    );
}

fn draw_stub(f: &mut Frame, area: Rect, page: Page) {
    let name = page.title();
    let body = match page {
        Page::Shipping => {
            "Shipping is a stub / opt-in GitHub placeholder \u{2014} not observed shipping data.\n\ncorrelation with sessions \u{2014} not a productivity score."
        }
        Page::Share => "Share to org defaults Off in v1 (UI only).",
        _ => {
            "Navigation stub in v1. Today + Insights are live.\n\naggregates only \u{2014} no raw prompts or code \u{2014} local-first"
        }
    };
    let lines = vec![
        Line::from(Span::styled(format!("{name} \u{2014} stub"), title_style())),
        Line::from(""),
        Line::from(Span::styled(body, muted())),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(panel(name)),
        area,
    );
}
