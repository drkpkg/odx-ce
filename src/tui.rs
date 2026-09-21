use crate::ui::Ui;
use console::style;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseEvent, MouseEventKind,
};
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
/// Lines moved per wheel notch.
const SCROLL_STEP: usize = 3;
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
    /// Emitted by odx itself (restart markers), never parsed from odoo-bin output.
    Notice,
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
            LogLevel::Notice => Color::Cyan,
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
    /// Lowercased copy of `raw`, built once here so the search filter doesn't
    /// re-lowercase the whole buffer on every redraw (~7 frames/s x 10k lines).
    lower: String,
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
            lower: raw.to_lowercase(),
        }
    }

    /// A line written by odx itself rather than by odoo-bin.
    fn notice(msg: impl Into<String>) -> Self {
        let raw = msg.into();
        let lower = raw.to_lowercase();
        Self {
            raw,
            level: LogLevel::Notice,
            lower,
        }
    }
}

/// Colorize a raw Odoo log line by level for the non-TUI fallback path (piping,
/// `--json`, `--no-progress`, `--plain`, non-TTY). Returns the line unchanged when
/// `ui` says colors are off, so callers don't need to branch on that themselves.
///
/// Callers colorizing a whole log stream should resolve the flag once and use
/// [`colorize_with`] instead of asking `ui` per line.
pub fn colorize(ui: &Ui, line: &OdooLogLine) -> String {
    colorize_with(ui.use_color(), line)
}

/// [`colorize`] with the color decision already made by the caller.
pub fn colorize_with(use_color: bool, line: &OdooLogLine) -> String {
    if !use_color {
        return line.raw.clone();
    }
    match line.level {
        LogLevel::Debug => style(&line.raw).dim().to_string(),
        LogLevel::Warning => style(&line.raw).yellow().to_string(),
        LogLevel::Error => style(&line.raw).red().bold().to_string(),
        LogLevel::Critical => style(&line.raw).magenta().bold().to_string(),
        LogLevel::Notice => style(&line.raw).cyan().to_string(),
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
        // odx's own markers stay visible whatever the filter is set to, otherwise a
        // restart would silently vanish from a filtered view.
        if matches!(level, LogLevel::Notice) {
            return true;
        }
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
            .filter(|l| needle.is_empty() || l.lower.contains(&needle))
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
    /// Largest useful `scroll` for the last rendered frame. `render` keeps it up to
    /// date so key handling can clamp instead of letting `scroll` run away past the
    /// top of the buffer (which would make Down/'j' inert until it counted back down).
    max_scroll: usize,
    /// Visible log rows in the last rendered frame, for PageUp/PageDown.
    body_height: usize,
    should_quit: bool,
    /// Whether wheel events are captured. Capturing them costs the terminal's own
    /// selection (most emulators then need Shift held to select text), so 'm' hands
    /// the mouse back when the user wants to copy a stack trace.
    mouse_capture: bool,
    /// Set by 'm'; the event loop applies it, since it talks to the terminal.
    toggle_mouse: bool,
    /// Set by the 'r' key; the event loop performs the restart (it owns the child).
    restart_requested: bool,
    restarts: usize,
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
            max_scroll: 0,
            body_height: 0,
            should_quit: false,
            mouse_capture: true,
            toggle_mouse: false,
            restart_requested: false,
            restarts: 0,
            running: true,
            start: now,
            rate: 0.0,
            rate_checked_at: now,
            rate_checked_count: 0,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // Raw mode means the terminal no longer turns Ctrl+C into SIGINT, so it has to
        // be handled here — including while typing a search query, where it used to be
        // swallowed as a literal 'c'.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

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
                // Modified keys (Ctrl+D, Alt+F, ...) are chords, not text input.
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                {
                    buf.push(c)
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('r') => self.restart_requested = true,
            KeyCode::Char('l') => self.filter = self.filter.next(),
            KeyCode::Char('/') => self.search_input = Some(String::new()),
            KeyCode::Esc => self.search_query.clear(),
            KeyCode::Char('m') => self.toggle_mouse = true,
            KeyCode::Char('g') => self.scroll = self.max_scroll,
            KeyCode::Char('G') => self.scroll = 0,
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(1),
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(1),
            KeyCode::PageUp => self.scroll_up(self.page()),
            KeyCode::PageDown => self.scroll_down(self.page()),
            _ => {}
        }
    }

    fn handle_mouse(&mut self, ev: MouseEvent) {
        match ev.kind {
            MouseEventKind::ScrollUp => self.scroll_up(SCROLL_STEP),
            MouseEventKind::ScrollDown => self.scroll_down(SCROLL_STEP),
            _ => {}
        }
    }

    /// Scroll back towards older lines, never past the top of the buffer.
    fn scroll_up(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_add(lines).min(self.max_scroll);
    }

    /// Scroll towards the tail; 0 re-pins the view to the newest line.
    fn scroll_down(&mut self, lines: usize) {
        self.scroll = self.scroll.saturating_sub(lines);
    }

    /// One screenful, as of the last rendered frame.
    fn page(&self) -> usize {
        self.body_height.max(1)
    }

    /// Append a line from odx itself to the on-screen buffer.
    fn notice(&mut self, msg: impl Into<String>) {
        self.buffer.push(OdooLogLine::notice(msg));
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

fn render(frame: &mut Frame, app: &mut App, title: &str) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(frame.area());

    let visible = app.buffer.visible(app.filter, &app.search_query);
    let body_area = chunks[0];
    let body_height = body_area.height.saturating_sub(2) as usize;
    let max_scroll = visible.len().saturating_sub(body_height);
    app.max_scroll = max_scroll;
    app.body_height = body_height;
    // Clamp the stored value, not just this frame's copy: a filter change or evicted
    // lines can shrink the buffer under a scrolled-up view.
    app.scroll = app.scroll.min(max_scroll);
    let scroll = app.scroll;
    let end = visible.len().saturating_sub(scroll);
    let start = end.saturating_sub(body_height);

    let lines: Vec<Line> = visible[start..end]
        .iter()
        .map(|l| Line::styled(l.raw.clone(), Style::default().fg(l.level.color())))
        .collect();

    let keybinds = keybind_hint(body_area.width, title.chars().count());
    let mut block = Block::default()
        .borders(Borders::ALL)
        .title(Line::from(format!(" {} ", title)));
    if !keybinds.is_empty() {
        block = block.title(Line::from(format!(" {} ", keybinds)).right_aligned());
    }

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
        // Scrolled-up views stop following the tail while lines keep arriving; say so,
        // otherwise the dashboard just looks frozen.
        let follow = if app.scroll == 0 {
            Span::raw("")
        } else {
            Span::styled(
                format!("   paused -{} lines ([G] to resume)", app.scroll),
                Style::default().fg(Color::Yellow),
            )
        };
        let mut spans = vec![
            Span::raw(format!("filter: [{}]", app.filter.label())),
            Span::raw(format!("   {:.1} lines/s", app.rate)),
            Span::raw(format!("   uptime {}s", app.start.elapsed().as_secs())),
            Span::raw("   "),
            indicator,
            follow,
        ];
        if !app.mouse_capture {
            spans.push(Span::styled(
                "   mouse: off",
                Style::default().fg(Color::DarkGray),
            ));
        }
        if app.restarts > 0 {
            spans.push(Span::raw(format!("   restarts: {}", app.restarts)));
        }
        if !app.search_query.is_empty() {
            spans.push(Span::raw(format!("   search: {:?}", app.search_query)));
        }
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(status_line), status_area);
}

/// Longest keybind hint that still leaves room for the project title. Narrow
/// terminals used to get a hint that ate the title and was itself clipped mid-word.
fn keybind_hint(width: u16, title_len: usize) -> &'static str {
    const HINTS: [&str; 4] = [
        "[q]uit [r]estart [/]search [l]evel [m]ouse [g/G]top/bottom",
        "[q]uit [r]estart [/]search [l]evel",
        "[q]uit [r]estart",
        "",
    ];
    // 2 borders + the spaces padding each title.
    let available = (width as usize).saturating_sub(title_len + 6);
    HINTS
        .into_iter()
        .find(|h| h.chars().count() <= available)
        .unwrap_or("")
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>, String> {
    enable_raw_mode().map_err(|e| format!("Failed to enable raw mode: {}", e))?;
    // From here on every failure path has to undo raw mode (and the alternate screen)
    // before returning, or odx exits leaving the user with a terminal that no longer
    // echoes input.
    let mut stdout = io::stdout();
    if let Err(e) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
        restore_terminal_best_effort();
        return Err(format!("Failed to enter alternate screen: {}", e));
    }
    Terminal::new(CrosstermBackend::new(stdout)).map_err(|e| {
        restore_terminal_best_effort();
        format!("Failed to initialize terminal: {}", e)
    })
}

fn restore_terminal() -> Result<(), String> {
    disable_raw_mode().map_err(|e| format!("Failed to disable raw mode: {}", e))?;
    execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen)
        .map_err(|e| format!("Failed to leave alternate screen: {}", e))?;
    Ok(())
}

fn restore_terminal_best_effort() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
}

/// Hand the mouse to the terminal (so the user can select and copy text) or take it
/// back for wheel scrolling.
fn set_mouse_capture(enabled: bool) -> Result<(), String> {
    let mut stdout = io::stdout();
    if enabled {
        execute!(stdout, EnableMouseCapture)
            .map_err(|e| format!("Failed to enable mouse capture: {}", e))
    } else {
        execute!(stdout, DisableMouseCapture)
            .map_err(|e| format!("Failed to disable mouse capture: {}", e))
    }
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

/// Spawns odoo-bin and wires the new process's stdout/stderr into the dashboard's
/// channel and the session log. Used for the first start and for every restart, so
/// both go through exactly the same plumbing.
struct Supervisor<'a> {
    spawn: &'a mut dyn FnMut() -> Result<Child, String>,
    tx: Sender<String>,
    log: Arc<Mutex<fs::File>>,
}

impl Supervisor<'_> {
    fn start(&mut self) -> Result<Child, String> {
        let mut child = (self.spawn)()?;
        let stdout: ChildStdout = child
            .stdout
            .take()
            .ok_or("Failed to capture odoo-bin stdout")?;
        let stderr: ChildStderr = child
            .stderr
            .take()
            .ok_or("Failed to capture odoo-bin stderr")?;
        spawn_reader(stdout, self.tx.clone(), Some(self.log.clone()));
        spawn_reader(stderr, self.tx.clone(), Some(self.log.clone()));
        Ok(child)
    }

    /// Write a marker straight to the session log, so restarts are visible when
    /// reading run.log later instead of two runs blurring into one.
    fn note(&self, msg: &str) {
        if let Ok(mut f) = self.log.lock() {
            let _ = writeln!(f, "{}", msg);
        }
    }
}

/// Signal the child's whole process group on Unix, falling back to the single pid if
/// it isn't a group leader. Odoo in prefork mode (`workers > 0`) forks HTTP/cron
/// workers that hold the listening socket, so signalling only the master leaves them
/// running and the next `odx run` fails with "Address already in use". Callers that
/// spawn the child themselves should put it in its own group (see `commands::run`).
#[cfg(unix)]
fn signal_process_group(child: &Child, signal: libc::c_int) {
    let pid = child.id() as libc::pid_t;
    unsafe {
        // Negative pid = "every process in the group with that id".
        if libc::kill(-pid, signal) == -1 {
            libc::kill(pid, signal);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopResult {
    AlreadyExited,
    Stopped,
    /// The graceful window expired and the process group had to be killed.
    Forced,
}

/// Best-effort stop: SIGINT first so odoo-bin/werkzeug can shut down cleanly (this is
/// what happens today when Ctrl+C reaches the child directly through the terminal's
/// process group); fall back to a hard kill if it doesn't exit in time. Returns what
/// it had to do instead of reporting it, because the caller may be inside the
/// alternate screen (a restart) where printing would corrupt the display.
#[cfg(unix)]
fn stop_child(child: &mut Child) -> StopResult {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return StopResult::AlreadyExited;
    }

    signal_process_group(child, libc::SIGINT);
    let deadline = Instant::now() + GRACEFUL_STOP_TIMEOUT;
    while Instant::now() < deadline {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return StopResult::Stopped;
        }
        thread::sleep(Duration::from_millis(100));
    }

    // Take the workers down with the master; `Child::kill()` only covers the process
    // we spawned.
    signal_process_group(child, libc::SIGKILL);
    let _ = child.kill();
    let _ = child.wait();
    StopResult::Forced
}

/// Windows has no equivalent to a graceful SIGINT here, so it goes straight to
/// `Child::kill()` — this mirrors the existing Unix-only signal handling in
/// `commands/test.rs`.
#[cfg(not(unix))]
fn stop_child(child: &mut Child) -> StopResult {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return StopResult::AlreadyExited;
    }
    let _ = child.kill();
    let _ = child.wait();
    StopResult::Stopped
}

/// How the dashboard session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The user quit while odoo-bin was still running; the caller must stop it.
    UserQuit,
    /// odoo-bin exited on its own. `code` is `None` when it was killed by a signal.
    ChildExited { code: Option<i32> },
}

fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    child: &mut Child,
    rx: &Receiver<String>,
    title: &str,
    supervisor: &mut Supervisor<'_>,
) -> Result<Outcome, String> {
    let mut app = App::new();
    let mut last_tick = Instant::now();
    // Remembered because `try_wait()` only reports the status once, and the dashboard
    // deliberately stays open after the child dies so the user can read the tail.
    let mut child_exit: Option<Option<i32>> = None;
    // A restart that could not spawn a replacement leaves nothing running; the
    // dashboard stays open so the reason is readable, and the error surfaces on quit.
    let mut restart_error: Option<String> = None;

    loop {
        terminal
            .draw(|f| render(f, &mut app, title))
            .map_err(|e| format!("Failed to draw TUI: {}", e))?;

        while let Ok(line) = rx.try_recv() {
            app.buffer.push(OdooLogLine::parse(&line));
        }
        app.maybe_refresh_rate();

        let timeout = TICK.saturating_sub(last_tick.elapsed());
        if event::poll(timeout).map_err(|e| format!("Failed to poll input: {}", e))? {
            match event::read().map_err(|e| format!("Failed to read input: {}", e))? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key),
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                _ => {}
            }
        }

        if app.toggle_mouse {
            app.toggle_mouse = false;
            let wanted = !app.mouse_capture;
            match set_mouse_capture(wanted) {
                Ok(()) => {
                    app.mouse_capture = wanted;
                    app.notice(if wanted {
                        "odx: mouse capture on (wheel scrolls the log)"
                    } else {
                        "odx: mouse capture off (terminal selection restored)"
                    });
                }
                // Not fatal — the dashboard is still usable from the keyboard.
                Err(e) => app.notice(format!("odx: {}", e)),
            }
        }
        if last_tick.elapsed() >= TICK {
            last_tick = Instant::now();
        }

        if app.running {
            if let Ok(Some(status)) = child.try_wait() {
                app.running = false;
                child_exit = Some(status.code());
                // Keep the dashboard open so the user can see why it stopped instead
                // of the window disappearing out from under them.
            }
        }

        if app.restart_requested {
            app.restart_requested = false;
            app.notice("odx: restarting odoo-bin...");
            // Draw once before the stop, which can block for the graceful timeout.
            terminal
                .draw(|f| render(f, &mut app, title))
                .map_err(|e| format!("Failed to draw TUI: {}", e))?;

            supervisor.note("=== odx: restart requested ===");
            if stop_child(child) == StopResult::Forced {
                app.notice("odx: odoo-bin did not stop gracefully, forced shutdown");
            }

            match supervisor.start() {
                Ok(new_child) => {
                    *child = new_child;
                    app.running = true;
                    app.restarts += 1;
                    child_exit = None;
                    restart_error = None;
                    app.notice("odx: odoo-bin restarted");
                }
                Err(e) => {
                    app.running = false;
                    child_exit = None;
                    app.notice(format!("odx: restart failed: {}", e));
                    app.notice("odx: press 'r' to try again, 'q' to quit");
                    restart_error = Some(e);
                }
            }
        }

        if app.should_quit {
            if let Some(e) = restart_error {
                return Err(e);
            }
            return Ok(match child_exit {
                Some(code) => Outcome::ChildExited { code },
                None => Outcome::UserQuit,
            });
        }
    }
}

/// Run the interactive log dashboard, taking ownership of the odoo-bin process until
/// the user quits (or it exits on its own). `spawn` starts odoo-bin; it is called once
/// up front and again for every restart requested with 'r', so anything that should be
/// re-read on restart (the addons path, odoo.conf.local) belongs inside it. The full,
/// unfiltered log of every start is mirrored to `session_log_path`.
pub fn run<S>(mut spawn: S, session_log_path: PathBuf, title: String, ui: &Ui) -> Result<(), String>
where
    S: FnMut() -> Result<Child, String>,
{
    let log_file = open_session_log(&session_log_path)?;
    let (tx, rx) = mpsc::channel::<String>();
    let mut supervisor = Supervisor {
        spawn: &mut spawn,
        tx,
        log: log_file,
    };

    let mut child = supervisor.start()?;

    // Installed before the terminal is touched so a panic inside `setup_terminal`
    // itself still restores the screen.
    install_panic_hook();
    let mut terminal = setup_terminal()?;

    let outcome = event_loop(&mut terminal, &mut child, &rx, &title, &mut supervisor);
    // Kept as a value rather than `?`-ed: the child has to be stopped even when the
    // terminal can no longer be restored, or odx exits leaving odoo-bin holding the
    // HTTP port.
    let restored = restore_terminal();

    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(e) => {
            stop_child(&mut child);
            return Err(e);
        }
    };

    if outcome == Outcome::UserQuit {
        ui.info(format!(
            "Stopping odoo-bin (full log: {})...",
            session_log_path.display()
        ));
    }
    if stop_child(&mut child) == StopResult::Forced {
        ui.warn("odoo-bin did not stop gracefully in time, forced shutdown");
    }
    restored?;

    match outcome {
        Outcome::UserQuit => Ok(()),
        Outcome::ChildExited { code: Some(0) } => Ok(()),
        Outcome::ChildExited { code: Some(code) } => {
            Err(format!("odoo-bin exited with code {}", code))
        }
        Outcome::ChildExited { code: None } => {
            Err("odoo-bin was terminated by a signal".to_string())
        }
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

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn scrolling_up_is_clamped_to_the_last_rendered_maximum() {
        let mut app = App::new();
        app.max_scroll = 3;

        for _ in 0..10 {
            app.handle_key(key(KeyCode::Up));
        }
        assert_eq!(
            app.scroll, 3,
            "scroll must not run past the top of the buffer"
        );

        app.handle_key(key(KeyCode::Down));
        assert_eq!(
            app.scroll, 2,
            "a single Down must move the view immediately"
        );
    }

    #[test]
    fn jump_to_top_then_scroll_down_moves_the_view() {
        let mut app = App::new();
        app.max_scroll = 5;

        app.handle_key(key(KeyCode::Char('g')));
        assert_eq!(app.scroll, 5);

        app.handle_key(key(KeyCode::Char('j')));
        assert_eq!(app.scroll, 4);

        app.handle_key(key(KeyCode::Char('G')));
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn keybind_hint_shrinks_before_it_eats_the_title() {
        let title = "odx run — my_project (Odoo 18.0)".chars().count();

        assert!(keybind_hint(160, title).contains("[m]ouse"));
        assert!(keybind_hint(100, title).starts_with("[q]uit [r]estart"));
        assert_eq!(keybind_hint(40, title), "", "no room: title wins");

        for width in [40u16, 60, 80, 100, 120, 160] {
            let hint = keybind_hint(width, title);
            assert!(
                hint.chars().count() + title + 6 <= width as usize || hint.is_empty(),
                "hint {hint:?} does not fit in {width} columns"
            );
        }
    }

    fn wheel(kind: MouseEventKind) -> MouseEvent {
        MouseEvent {
            kind,
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn wheel_scrolls_by_a_step_and_stops_at_both_ends() {
        let mut app = App::new();
        app.max_scroll = 10;

        app.handle_mouse(wheel(MouseEventKind::ScrollUp));
        assert_eq!(app.scroll, SCROLL_STEP);

        app.handle_mouse(wheel(MouseEventKind::ScrollDown));
        assert_eq!(app.scroll, 0, "back to the tail");

        for _ in 0..20 {
            app.handle_mouse(wheel(MouseEventKind::ScrollUp));
        }
        assert_eq!(app.scroll, 10, "must not scroll past the oldest line");

        for _ in 0..20 {
            app.handle_mouse(wheel(MouseEventKind::ScrollDown));
        }
        assert_eq!(app.scroll, 0, "must re-pin to the newest line");
    }

    #[test]
    fn other_mouse_events_do_not_move_the_view() {
        let mut app = App::new();
        app.max_scroll = 10;
        app.scroll = 4;

        app.handle_mouse(wheel(MouseEventKind::Moved));

        assert_eq!(app.scroll, 4);
    }

    #[test]
    fn page_keys_scroll_by_the_rendered_height() {
        let mut app = App::new();
        app.max_scroll = 100;
        app.body_height = 20;

        app.handle_key(key(KeyCode::PageUp));
        assert_eq!(app.scroll, 20);

        app.handle_key(key(KeyCode::PageDown));
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn m_requests_a_mouse_capture_toggle() {
        let mut app = App::new();
        assert!(app.mouse_capture, "wheel scrolling is on by default");

        app.handle_key(key(KeyCode::Char('m')));

        assert!(app.toggle_mouse);
        assert!(
            app.mouse_capture,
            "the event loop applies it, so state flips only once the terminal agrees"
        );
    }

    #[test]
    fn r_requests_a_restart_and_notices_survive_every_filter() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('r')));
        assert!(app.restart_requested);
        assert!(!app.should_quit, "restart must not end the session");

        // A restart marker has to stay visible even when the view is filtered to
        // errors only, otherwise the restart looks like nothing happened.
        app.notice("odx: restarting odoo-bin...");
        app.filter = LevelFilter::Error;
        let visible = app.buffer.visible(app.filter, "");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].level, LogLevel::Notice);
    }

    #[test]
    fn r_typed_into_a_search_query_does_not_restart() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('r')));

        assert!(!app.restart_requested);
        assert_eq!(app.search_input.as_deref(), Some("r"));
    }

    #[test]
    fn ctrl_c_quits_even_while_typing_a_search_query() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(key(KeyCode::Char('a')));

        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert!(app.should_quit, "Ctrl+C must quit from search input too");
        assert_eq!(
            app.search_input.as_deref(),
            Some("a"),
            "Ctrl+C must not be typed into the query"
        );
    }

    #[test]
    fn modified_keys_are_not_typed_into_the_search_query() {
        let mut app = App::new();
        app.handle_key(key(KeyCode::Char('/')));
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        app.handle_key(key(KeyCode::Char('x')));

        assert_eq!(app.search_input.as_deref(), Some("x"));
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
