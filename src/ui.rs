use console::{style, Term};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use inquire::Confirm;
use std::io;
use std::sync::OnceLock;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone)]
pub struct UiConfig {
    pub color: ColorMode,
    pub quiet: bool,
    pub json: bool,
    pub progress: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            color: ColorMode::Auto,
            quiet: false,
            json: false,
            progress: true,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Ui {
    cfg: UiConfig,
    /// `isatty` results are stable for the life of the process, so they are resolved
    /// once instead of per message — `odx run` asks for them on every streamed log line.
    stdout_tty: OnceLock<bool>,
    stderr_tty: OnceLock<bool>,
}

impl Ui {
    pub fn new(cfg: UiConfig) -> Self {
        Self {
            cfg,
            stdout_tty: OnceLock::new(),
            stderr_tty: OnceLock::new(),
        }
    }

    pub fn config(&self) -> &UiConfig {
        &self.cfg
    }

    pub fn is_stdout_tty(&self) -> bool {
        *self.stdout_tty.get_or_init(|| Term::stdout().is_term())
    }

    pub fn is_stderr_tty(&self) -> bool {
        *self.stderr_tty.get_or_init(|| Term::stderr().is_term())
    }

    pub fn use_color(&self) -> bool {
        match self.cfg.color {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => self.is_stdout_tty() && !self.cfg.json,
        }
    }

    pub fn use_progress(&self) -> bool {
        self.cfg.progress && !self.cfg.quiet && !self.cfg.json && self.is_stderr_tty()
    }

    pub fn info(&self, msg: impl AsRef<str>) {
        if self.cfg.quiet || self.cfg.json {
            return;
        }
        println!("{}", msg.as_ref());
    }

    /// Final, must-not-be-swallowed command output (a test summary, a result count).
    /// Unlike [`Ui::info`] it survives `--quiet`; under `--json` it is suppressed and
    /// the command is expected to emit a structured payload with [`Ui::json_line`].
    pub fn summary(&self, msg: impl AsRef<str>) {
        if self.cfg.json {
            return;
        }
        println!("{}", msg.as_ref());
    }

    /// Emit one machine-readable JSON line on stdout. No-op unless `--json` is set, so
    /// callers can keep the human and JSON paths side by side.
    pub fn json_line(&self, value: &serde_json::Value) {
        if !self.cfg.json {
            return;
        }
        println!("{}", value);
    }

    /// Pass a line of child-process output straight through to stdout/stderr, already
    /// formatted. The caller decides what `--quiet`/`--json` mean for that stream (see
    /// `commands::run`), which is why this does no filtering of its own.
    pub fn passthrough(&self, to_stderr: bool, msg: impl AsRef<str>) {
        if to_stderr {
            eprintln!("{}", msg.as_ref());
        } else {
            println!("{}", msg.as_ref());
        }
    }

    pub fn warn(&self, msg: impl AsRef<str>) {
        if self.cfg.json {
            return;
        }
        if self.use_color() {
            eprintln!("{}", style(msg.as_ref()).yellow());
        } else {
            eprintln!("{}", msg.as_ref());
        }
    }

    pub fn error(&self, msg: impl AsRef<str>) {
        if self.use_color() {
            eprintln!("{}", style(msg.as_ref()).red().bold());
        } else {
            eprintln!("{}", msg.as_ref());
        }
    }

    pub fn success(&self, msg: impl AsRef<str>) {
        if self.cfg.quiet || self.cfg.json {
            return;
        }
        if self.use_color() {
            println!("{}", style(msg.as_ref()).green());
        } else {
            println!("{}", msg.as_ref());
        }
    }

    pub fn heading(&self, title: impl AsRef<str>) {
        if self.cfg.quiet || self.cfg.json {
            return;
        }
        let t = title.as_ref();
        if self.use_color() {
            println!("{}", style(t).bold());
        } else {
            println!("{}", t);
        }
    }

    pub fn check(&self, ok: bool, label: impl AsRef<str>, details: Option<&str>) {
        if self.cfg.json {
            return;
        }
        let label = label.as_ref();
        let details = details.unwrap_or("");
        let suffix = if details.is_empty() {
            String::new()
        } else {
            format!(" {}", details)
        };

        if self.use_color() {
            let tag = if ok {
                style("OK").green().bold()
            } else {
                style("FAIL").red().bold()
            };
            println!("[{}] {}{}", tag, label, suffix);
        } else {
            let tag = if ok { "OK" } else { "FAIL" };
            println!("[{}] {}{}", tag, label, suffix);
        }
    }

    pub fn spinner(&self, msg: impl AsRef<str>) -> Spinner {
        if !self.use_progress() {
            return Spinner { pb: None };
        }

        let pb = ProgressBar::new_spinner();
        pb.set_draw_target(ProgressDrawTarget::stderr());
        pb.set_style(
            ProgressStyle::with_template("{spinner:.dim} {msg}")
                .unwrap()
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
        );
        pb.enable_steady_tick(Duration::from_millis(90));
        pb.set_message(msg.as_ref().to_string());
        Spinner { pb: Some(pb) }
    }

    pub fn progress_bar(&self, total: u64) -> Option<ProgressBar> {
        if !self.use_progress() {
            return None;
        }
        let pb = ProgressBar::new(total);
        pb.set_draw_target(ProgressDrawTarget::stderr());
        pb.set_style(
            ProgressStyle::with_template("{bar:40.cyan/blue} {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("=>-"),
        );
        Some(pb)
    }

    pub fn prompt_confirm(&self, message: impl AsRef<str>, default: bool) -> Result<bool, String> {
        if self.cfg.json {
            return Err("Prompts are disabled in --json mode".to_string());
        }
        if !Term::stdout().is_term() {
            return Err("Prompt requires an interactive terminal".to_string());
        }
        Confirm::new(message.as_ref())
            .with_default(default)
            .prompt()
            .map_err(|e| match e {
                inquire::InquireError::OperationCanceled => "Canceled".to_string(),
                inquire::InquireError::OperationInterrupted => "Interrupted".to_string(),
                other => format!("Prompt failed: {}", other),
            })
    }
}

pub struct Spinner {
    pb: Option<ProgressBar>,
}

impl Spinner {
    pub fn set_message(&self, msg: impl AsRef<str>) {
        if let Some(pb) = &self.pb {
            pb.set_message(msg.as_ref().to_string());
        }
    }

    pub fn finish_with_message(mut self, msg: impl AsRef<str>) {
        if let Some(pb) = self.pb.take() {
            pb.finish_with_message(msg.as_ref().to_string());
        }
    }

    pub fn finish_and_clear(mut self) {
        if let Some(pb) = self.pb.take() {
            pb.finish_and_clear();
        }
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        if let Some(pb) = self.pb.take() {
            pb.finish_and_clear();
        }
    }
}

pub fn is_broken_pipe(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::BrokenPipe
}
