use crate::ui::Ui;
use console::style;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::{Frame, Terminal};
use regex::Regex;
use std::collections::VecDeque;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

const MAX_LINES: usize = 10_000;
const TICK: Duration = Duration::from_millis(150);
const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warning,
    Error,
    Critical,
    Unknown,
}

impl LogLevel {
    fn color(self) -> Color {
        match self {
            LogLevel::Debug => Color::DarkGray,
            LogLevel::Info => Color::Gray,
            LogLevel::Warning => Color::Yellow,
            LogLevel::Error => Color::Red,
            LogLevel::Critical => Color::Magenta,
            LogLevel::Unknown => Color::Gray,
        }
    }
}

fn level_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Odoo's standard log line: "<date> <time> <pid> LEVEL <db> <module>: message"
    RE.get_or_init(|| {
        Regex::new(r"^\S+\s+\S+\s+\d+\s+(DEBUG|INFO|WARNING|ERROR|CRITICAL)\b").unwrap()
    })
}

#[derive(Debug, Clone)]
pub struct OdooLogLine {
    pub raw: String,
    pub level: LogLevel,
}

impl OdooLogLine {
    pub fn parse(raw: &str) -> Self {
        let level = level_regex()
            .captures(raw)
            .and_then(|c| c.get(1))
            .map(|m| match m.as_str() {
                "DEBUG" => LogLevel::Debug,
                "INFO" => LogLevel::Info,
                "WARNING" => LogLevel::Warning,
                "ERROR" => LogLevel::Error,
                "CRITICAL" => LogLevel::Critical,
                _ => LogLevel::Unknown,
            })
            .unwrap_or(LogLevel::Unknown);
        Self {
            raw: raw.to_string(),
            level,
        }
    }
}

/// Colorize a raw Odoo log line by level for the non-TUI fallback path (piping,
/// `--json`, `--no-progress`, `--plain`, non-TTY). Returns the line unchanged when
/// `ui` says colors are off, so callers don't need to branch on that themselves.
pub fn colorize(ui: &Ui, line: &OdooLogLine) -> String {
    if !ui.use_color() {
        return line.raw.clone();
    }
    match line.level {
        LogLevel::Debug => style(&line.raw).dim().to_string(),
        LogLevel::Warning => style(&line.raw).yellow().to_string(),
        LogLevel::Error => style(&line.raw).red().bold().to_string(),
        LogLevel::Critical => style(&line.raw).magenta().bold().to_string(),
        LogLevel::Info | LogLevel::Unknown => line.raw.clone(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LevelFilter {
    All,
    Info,
    Warning,
    Error,
}

impl LevelFilter {
    fn next(self) -> Self {
        match self {
            LevelFilter::All => LevelFilter::Info,
            LevelFilter::Info => LevelFilter::Warning,
            LevelFilter::Warning => LevelFilter::Error,
            LevelFilter::Error => LevelFilter::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            LevelFilter::All => "ALL",
            LevelFilter::Info => "INFO",
            LevelFilter::Warning => "WARN",
            LevelFilter::Error => "ERROR",
        }
    }

    fn matches(self, level: LogLevel) -> bool {
        match self {
            LevelFilter::All => true,
            LevelFilter::Info => !matches!(level, LogLevel::Debug),
            LevelFilter::Warning => matches!(
                level,
                LogLevel::Warning | LogLevel::Error | LogLevel::Critical
            ),
            LevelFilter::Error => matches!(level, LogLevel::Error | LogLevel::Critical),
        }
    }
}

struct LogBuffer {
    lines: VecDeque<OdooLogLine>,
    cap: usize,
    total_received: u64,
}

impl LogBuffer {
    fn new(cap: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(cap.min(1024)),
            cap,
            total_received: 0,
        }
    }

    fn push(&mut self, line: OdooLogLine) {
        self.total_received += 1;
        if self.lines.len() >= self.cap {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }

    fn visible(&self, filter: LevelFilter, query: &str) -> Vec<&OdooLogLine> {
        let needle = query.to_lowercase();
        self.lines
            .iter()
            .filter(|l| filter.matches(l.level))
            .filter(|l| needle.is_empty() || l.raw.to_lowercase().contains(&needle))
            .collect()
    }
}

struct App {
    buffer: LogBuffer,
    filter: LevelFilter,
    search_query: String,
    search_input: Option<String>,
    /// Lines scrolled up from the tail of the *currently visible* set. 0 = pinned to
    /// the bottom (tail -f style). Because this is a distance-from-tail rather than
    /// an absolute index, the view keeps its distance as new lines arrive instead of
    /// freezing on stale buffer positions once old lines get evicted.
    scroll: usize,
    should_quit: bool,
    running: bool,
    start: Instant,
    rate: f64,
    rate_checked_at: Instant,
    rate_checked_count: u64,
}

impl App {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            buffer: LogBuffer::new(MAX_LINES),
            filter: LevelFilter::All,
            search_query: String::new(),
            search_input: None,
            scroll: 0,
            should_quit: false,
            running: true,
            start: now,
            rate: 0.0,
            rate_checked_at: now,
            rate_checked_count: 0,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if let Some(buf) = self.search_input.as_mut() {
            match key.code {
                KeyCode::Enter => {
                    self.search_query = std::mem::take(buf);
                    self.search_input = None;
                }
                KeyCode::Esc => self.search_input = None,
                KeyCode::Backspace => {
                    buf.pop();
                }
                KeyCode::Char(c) => buf.push(c),
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Char('l') => self.filter = self.filter.next(),
            KeyCode::Char('/') => self.search_input = Some(String::new()),
            KeyCode::Esc => self.search_query.clear(),
            KeyCode::Char('g') => self.scroll = usize::MAX,
            KeyCode::Char('G') => self.scroll = 0,
            KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_add(1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll = self.scroll.saturating_sub(1),
            _ => {}
        }
    }

    fn maybe_refresh_rate(&mut self) {
        let elapsed = self.rate_checked_at.elapsed();
        if elapsed < Duration::from_secs(1) {
            return;
        }
        let delta = self.buffer.total_received - self.rate_checked_count;
        self.rate = delta as f64 / elapsed.as_secs_f64();
        self.rate_checked_at = Instant::now();
        self.rate_checked_count = self.buffer.total_received;
    }
}

fn render(frame: &mut Frame, app: &App, title: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(frame.area());

    let visible = app.buffer.visible(app.filter, &app.search_query);
    let body_area = chunks[0];
    let body_height = body_area.height.saturating_sub(2) as usize;
    let max_scroll = visible.len().saturating_sub(body_height);
    let scroll = app.scroll.min(max_scroll);
    let end = visible.len().saturating_sub(scroll);
    let start = end.saturating_sub(body_height);

    let lines: Vec<Line> = visible[start..end]
        .iter()
        .map(|l| Line::styled(l.raw.clone(), Style::default().fg(l.level.color())))
        .collect();

    let keybinds = "[q]uit [/]search [l]evel [g/G]top/bottom";
    let block = Block::default()
        .borders(Borders::ALL)
        .title(Line::from(format!(" {} ", title)))
        .title(Line::from(format!(" {} ", keybinds)).right_aligned());

    frame.render_widget(Paragraph::new(Text::from(lines)).block(block), body_area);

    let status_area = chunks[1];
    let status_line = if let Some(buf) = &app.search_input {
        Line::from(vec![
            Span::styled("/", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(buf.as_str()),
            Span::raw("_"),
        ])
    } else {
        let indicator = if app.running {
            Span::styled("running", Style::default().fg(Color::Green))
        } else {
            Span::styled("stopped", Style::default().fg(Color::Red))
        };
        let mut spans = vec![
            Span::raw(format!("filter: [{}]", app.filter.label())),
            Span::raw(format!("   {:.1} lines/s", app.rate)),
            Span::raw(format!(
                "   uptime {}s",
                app.start.elapsed().as_secs()
            )),
            Span::raw("   "),
            indicator,
        ];
        if !app.search_query.is_empty() {
            spans.push(Span::raw(format!("   search: {:?}", app.search_query)));
        }
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(status_line), status_area);
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>, String> {
    enable_raw_mode().map_err(|e| format!("Failed to enable raw mode: {}", e))?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)
        .map_err(|e| format!("Failed to enter alternate screen: {}", e))?;
    Terminal::new(CrosstermBackend::new(stdout))
        .map_err(|e| format!("Failed to initialize terminal: {}", e))
}

fn restore_terminal() -> Result<(), String> {
    disable_raw_mode().map_err(|e| format!("Failed to disable raw mode: {}", e))?;
    execute!(io::stdout(), LeaveAlternateScreen)
        .map_err(|e| format!("Failed to leave alternate screen: {}", e))?;
    Ok(())
}

fn restore_terminal_best_effort() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen);
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal_best_effort();
        previous(info);
    }));
}

fn spawn_reader<R>(stream: R, tx: Sender<String>, log_file: Option<Arc<Mutex<fs::File>>>)
where
    R: io::Read + Send + 'static,
{
    thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(f) = &log_file {
                if let Ok(mut f) = f.lock() {
                    let _ = writeln!(f, "{}", line);
                }
            }
            if tx.send(line).is_err() {
                break;
            }
        }
    });
}

fn open_session_log(path: &Path) -> Result<Arc<Mutex<fs::File>>, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }
    let file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
    Ok(Arc::new(Mutex::new(file)))
}

/// Best-effort stop: SIGINT first so odoo-bin/werkzeug can shut down cleanly (this is
/// what happens today when Ctrl+C reaches the child directly through the terminal's
/// process group); fall back to a hard kill if it doesn't exit in time. Windows has no
/// equivalent to a graceful SIGINT here, so it goes straight to `Child::kill()` — this
/// mirrors the existing Unix-only signal handling in `commands/test.rs`.
fn stop_child(child: &mut Child, ui: &Ui) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }

    #[cfg(unix)]
    {
        unsafe {
            libc::kill(child.id() as libc::pid_t, libc::SIGINT);
        }
        let deadline = Instant::now() + GRACEFUL_STOP_TIMEOUT;
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        ui.warn("odoo-bin did not stop gracefully in time, forcing shutdown...");
    }

    let _ = child.kill();
    let _ = child.wait();
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    child: &mut Child,
    rx: Receiver<String>,
    title: &str,
) -> Result<Option<i32>, String> {
    let mut app = App::new();
    let mut last_tick = Instant::now();

    loop {
        terminal
            .draw(|f| render(f, &app, title))
            .map_err(|e| format!("Failed to draw TUI: {}", e))?;

        while let Ok(line) = rx.try_recv() {
            app.buffer.push(OdooLogLine::parse(&line));
        }
        app.maybe_refresh_rate();

        let timeout = TICK.saturating_sub(last_tick.elapsed());
        if event::poll(timeout).map_err(|e| format!("Failed to poll input: {}", e))? {
            if let Event::Key(key) = event::read().map_err(|e| format!("Failed to read input: {}", e))? {
                if key.kind == KeyEventKind::Press {
                    app.handle_key(key);
                }
            }
        }
        if last_tick.elapsed() >= TICK {
            last_tick = Instant::now();
        }

        if app.running {
            if let Ok(Some(status)) = child.try_wait() {
                app.running = false;
                if app.should_quit {
                    return Ok(status.code());
                }
                // Keep the dashboard open so the user can see why it stopped instead
                // of the window disappearing out from under them.
            }
        }

        if app.should_quit {
            return Ok(None);
        }
    }
}

/// Run the interactive log dashboard for a child process, taking ownership of it
/// until the user quits (or the child exits on its own). The full, unfiltered log is
/// always mirrored to `session_log_path` regardless of what's filtered on screen.
pub fn run(mut child: Child, session_log_path: PathBuf, title: String, ui: &Ui) -> Result<(), String> {
    let stdout_pipe: ChildStdout = child
        .stdout
        .take()
        .ok_or("Failed to capture odoo-bin stdout")?;
    let stderr_pipe: ChildStderr = child
        .stderr
        .take()
        .ok_or("Failed to capture odoo-bin stderr")?;

    let log_file = open_session_log(&session_log_path)?;
    let (tx, rx) = mpsc::channel::<String>();
    spawn_reader(stdout_pipe, tx.clone(), Some(log_file.clone()));
    spawn_reader(stderr_pipe, tx, Some(log_file));

    let mut terminal = setup_terminal()?;
    install_panic_hook();

    let outcome = event_loop(&mut terminal, &mut child, rx, &title);

    restore_terminal()?;

    let already_exited = match outcome {
        Ok(code) => code,
        Err(e) => {
            stop_child(&mut child, ui);
            return Err(e);
        }
    };

    if already_exited.is_none() {
        ui.info(format!(
            "Stopping odoo-bin (full log: {})...",
            session_log_path.display()
        ));
    }
    stop_child(&mut child, ui);

    match already_exited {
        Some(code) if code != 0 => Err(format!("odoo-bin exited with code {}", code)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_levels() {
        let cases = [
            (
                "2026-05-23 23:49:13,986 991887 INFO db odoo.modules.loading: loaded",
                LogLevel::Info,
            ),
            (
                "2026-05-23 23:49:13,986 991887 WARNING db mail.models: outdated cache",
                LogLevel::Warning,
            ),
            (
                "2026-05-23 23:49:13,986 991887 ERROR db odoo.sql_db: could not connect",
                LogLevel::Error,
            ),
            (
                "2026-05-23 23:49:13,986 991887 CRITICAL db odoo: fatal",
                LogLevel::Critical,
            ),
            (
                "2026-05-23 23:49:13,986 991887 DEBUG db odoo: verbose",
                LogLevel::Debug,
            ),
        ];
        for (line, expected) in cases {
            assert_eq!(OdooLogLine::parse(line).level, expected, "line: {line}");
        }
    }

    #[test]
    fn parses_unrecognized_line_as_unknown() {
        let line = OdooLogLine::parse("not an odoo log line at all");
        assert_eq!(line.level, LogLevel::Unknown);
    }

    #[test]
    fn level_filter_cycles_through_all_variants() {
        let mut f = LevelFilter::All;
        let mut seen = vec![f];
        for _ in 0..3 {
            f = f.next();
            seen.push(f);
        }
        assert_eq!(f.next(), LevelFilter::All, "cycle should be closed");
        assert_eq!(
            seen,
            vec![
                LevelFilter::All,
                LevelFilter::Info,
                LevelFilter::Warning,
                LevelFilter::Error
            ]
        );
    }

    #[test]
    fn level_filter_matches_are_inclusive_upward() {
        assert!(LevelFilter::All.matches(LogLevel::Debug));
        assert!(!LevelFilter::Info.matches(LogLevel::Debug));
        assert!(LevelFilter::Info.matches(LogLevel::Warning));
        assert!(!LevelFilter::Warning.matches(LogLevel::Info));
        assert!(LevelFilter::Error.matches(LogLevel::Critical));
        assert!(!LevelFilter::Error.matches(LogLevel::Warning));
    }

    #[test]
    fn log_buffer_evicts_oldest_when_over_capacity() {
        let mut buf = LogBuffer::new(3);
        for i in 0..5 {
            buf.push(OdooLogLine::parse(&format!(
                "2026-01-01 00:00:00,000 1 INFO db mod: line{i}"
            )));
        }
        let visible = buf.visible(LevelFilter::All, "");
        assert_eq!(visible.len(), 3);
        assert!(visible[0].raw.contains("line2"));
        assert!(visible[2].raw.contains("line4"));
        assert_eq!(buf.total_received, 5);
    }

    #[test]
    fn log_buffer_search_filters_case_insensitively() {
        let mut buf = LogBuffer::new(10);
        buf.push(OdooLogLine::parse(
            "2026-01-01 00:00:00,000 1 INFO db mod: Loading Modules",
        ));
        buf.push(OdooLogLine::parse(
            "2026-01-01 00:00:00,000 1 ERROR db mod: connection refused",
        ));
        let visible = buf.visible(LevelFilter::All, "MODULES");
        assert_eq!(visible.len(), 1);
        assert!(visible[0].raw.contains("Loading Modules"));
    }

    #[test]
    fn log_buffer_search_and_level_filter_combine() {
        let mut buf = LogBuffer::new(10);
        buf.push(OdooLogLine::parse(
            "2026-01-01 00:00:00,000 1 INFO db mod: connection ok",
        ));
        buf.push(OdooLogLine::parse(
            "2026-01-01 00:00:00,000 1 ERROR db mod: connection refused",
        ));
        let visible = buf.visible(LevelFilter::Error, "connection");
        assert_eq!(visible.len(), 1);
        assert!(visible[0].raw.contains("refused"));
    }
}
