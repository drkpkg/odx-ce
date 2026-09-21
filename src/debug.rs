//! DAP (Debug Adapter Protocol) support, always on.
//!
//! Every Odoo process odx starts gets a [debugpy](https://github.com/microsoft/debugpy)
//! listener on loopback, so any DAP client (VS Code, Cursor, nvim-dap, PyCharm) can
//! attach without odx handing over the terminal — which it cannot do anyway, since the
//! `odx run` dashboard owns stdin in raw mode.
//!
//! The listener is installed through the *environment*, not by wrapping the command
//! line with `python -m debugpy ...`. Odoo restarts itself with
//! `os.execve(sys.executable, stripped_sys_argv(), os.environ)` (both on `--dev=reload`
//! and when a module install asks for a restart), and that re-exec keeps the
//! environment but drops any launcher prefix — an argv wrapper would silently lose the
//! debugger on the first reload. A `sitecustomize` module on `PYTHONPATH` survives it.

use std::fs;
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Loopback only. A DAP port is remote code execution for anyone who can reach it.
pub const HOST: &str = "127.0.0.1";
pub const DEFAULT_PORT: u16 = 5678;
/// How many ports to try past the requested one before giving up, so a second project
/// (or a leftover process) doesn't leave you without a debugger.
const PORT_SCAN: u16 = 10;

const SHIM_DIR: &str = ".testing/debug";
const SHIM_FILE: &str = "sitecustomize.py";

#[derive(Debug, Clone)]
pub struct DebugSetup {
    pub host: String,
    pub port: u16,
    pub wait: bool,
}

impl DebugSetup {
    /// `host:port`, for logs and IDE attach configuration.
    pub fn address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Environment for an Odoo process: the shim on `PYTHONPATH`, plus the settings it
    /// reads. `PYTHONBREAKPOINT` routes plain `breakpoint()` calls in addon code to the
    /// attached client instead of to a pdb prompt nobody can see. `odoo_bin` tells the
    /// shim which process is the one to debug — everything else that inherits this
    /// environment (debugpy's adapter, pip, helpers) must keep its hands off the port.
    pub fn env(&self, shim_dir: &Path, odoo_bin: &Path) -> Vec<(String, String)> {
        let mut python_path = shim_dir.to_string_lossy().to_string();
        if let Ok(existing) = std::env::var("PYTHONPATH") {
            if !existing.is_empty() {
                let sep = if cfg!(windows) { ';' } else { ':' };
                python_path = format!("{}{}{}", python_path, sep, existing);
            }
        }

        vec![
            ("PYTHONPATH".to_string(), python_path),
            (
                "ODX_DEBUG_TARGET".to_string(),
                odoo_bin.to_string_lossy().to_string(),
            ),
            ("ODX_DEBUG_HOST".to_string(), self.host.clone()),
            ("ODX_DEBUG_PORT".to_string(), self.port.to_string()),
            (
                "ODX_DEBUG_WAIT".to_string(),
                if self.wait { "1" } else { "0" }.to_string(),
            ),
            (
                "PYTHONBREAKPOINT".to_string(),
                "debugpy.breakpoint".to_string(),
            ),
            // Silences debugpy's "frozen modules" warning on Python 3.11+.
            (
                "PYDEVD_DISABLE_FILE_VALIDATION".to_string(),
                "1".to_string(),
            ),
        ]
    }
}

/// Write the shim and pick a usable port. `requested` comes from `--debug-port`;
/// `ODX_DEBUG_PORT` in the environment is used when no flag is given.
pub fn prepare(
    project_root: &Path,
    requested: Option<u16>,
    wait: bool,
) -> Result<(DebugSetup, PathBuf), String> {
    let shim_dir = write_shim(project_root)?;
    let preferred = requested
        .or_else(|| {
            std::env::var("ODX_DEBUG_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
        })
        .unwrap_or(DEFAULT_PORT);

    let setup = DebugSetup {
        host: HOST.to_string(),
        port: pick_port(preferred),
        wait,
    };
    Ok((setup, shim_dir))
}

/// First free port at or after `preferred`. Falls back to `preferred` itself when the
/// whole range is busy: the shim reports the bind failure and Odoo still starts.
fn pick_port(preferred: u16) -> u16 {
    (preferred..preferred.saturating_add(PORT_SCAN))
        .find(|p| port_is_free(*p))
        .unwrap_or(preferred)
}

fn port_is_free(port: u16) -> bool {
    TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)).is_ok()
}

/// Wait for a port to become bindable again, used before a dashboard restart: the
/// previous process may still be holding the listener for a few milliseconds, and the
/// shim only gets one attempt.
pub fn wait_for_port_free(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if port_is_free(port) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Is debugpy importable by this interpreter?
pub fn is_available(python: &str) -> bool {
    Command::new(python)
        .args(["-c", "import debugpy"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn write_shim(project_root: &Path) -> Result<PathBuf, String> {
    let dir = project_root.join(SHIM_DIR);
    fs::create_dir_all(&dir).map_err(|e| format!("Failed to create {}: {}", dir.display(), e))?;
    let path = dir.join(SHIM_FILE);
    // Rewritten every run so an odx upgrade can't leave a stale shim behind.
    fs::write(&path, SHIM_SOURCE)
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
    Ok(dir)
}

/// VS Code / Cursor attach configuration for this project.
pub fn launch_json(setup: &DebugSetup) -> String {
    format!(
        r#"{{
  "version": "0.2.0",
  "configurations": [
    {{
      "name": "odx: attach to Odoo",
      "type": "debugpy",
      "request": "attach",
      "connect": {{ "host": "{host}", "port": {port} }},
      "pathMappings": [{{ "localRoot": "${{workspaceFolder}}", "remoteRoot": "${{workspaceFolder}}" }}],
      "justMyCode": false
    }}
  ]
}}
"#,
        host = setup.host,
        port = setup.port
    )
}

/// Imported by CPython at startup for every process odx launches with the shim
/// directory on `PYTHONPATH`. Best effort throughout: a debugger that cannot start
/// must never stop the server from starting.
///
/// Two details are load-bearing, both established by testing against debugpy 1.8:
/// * `debugpy.configure(subProcess=False)` — pydevd otherwise patches `os.exec*` so
///   child processes re-attach to the session. Odoo restarts itself *in place* with
///   `os.execve`, and the patched call relaunches it under pydevd pointing at the
///   adapter of the process that just went away: the restart then dies with
///   `ConnectionRefusedError` instead of coming back up.
/// * the retry loop — `debugpy.listen` runs the DAP endpoint in a separate adapter
///   process that releases the port only once the old server is gone, so the first
///   bind after a restart can lose the race.
const SHIM_SOURCE: &str = r#"# Generated by odx -- do not edit; rewritten on every run.
#
# Starts a debugpy (DAP) listener in every Odoo process odx launches. It travels in
# PYTHONPATH rather than on the command line so that it survives Odoo restarting
# itself with os.execve(), which keeps the environment but drops any launcher prefix.

import os
import sys

_started = False


def _log(message):
    sys.stderr.write("odx: %s\n" % message)
    sys.stderr.flush()


def _is_target():
    """Only the Odoo process debugs itself.

    Everything else that inherits PYTHONPATH -- debugpy's own adapter process, pip,
    any helper -- must not try to bind the port.
    """
    target = os.environ.get("ODX_DEBUG_TARGET")
    if not target:
        return False
    argv0 = sys.argv[0] if sys.argv else ""
    if not argv0:
        return False
    return os.path.abspath(argv0) == os.path.abspath(target) or os.path.basename(
        argv0
    ) == os.path.basename(target)


def _listen(debugpy, host, port, attempts=15, delay=0.2):
    import time

    last = None
    for _ in range(attempts):
        try:
            debugpy.listen((host, port))
            return None
        except Exception as exc:
            last = exc
            time.sleep(delay)
    return last


def _start_debugger():
    global _started
    if _started or not _is_target():
        return
    _started = True

    port = os.environ.get("ODX_DEBUG_PORT")
    if not port:
        return
    host = os.environ.get("ODX_DEBUG_HOST", "127.0.0.1")

    try:
        import debugpy
    except ImportError:
        _log("debugpy not installed, debugger disabled (run 'odx install')")
        return

    try:
        # Keep pydevd away from os.exec*/fork: Odoo restarts itself in place.
        debugpy.configure(subProcess=False)
    except Exception as exc:
        _log("could not configure debugpy: %s" % exc)

    failure = _listen(debugpy, host, int(port))
    if failure is not None:
        _log("debugger disabled: %s" % failure)
        return

    _log("debugger listening on %s:%s (DAP, pid %s)" % (host, port, os.getpid()))

    if os.environ.get("ODX_DEBUG_WAIT") == "1":
        _log("waiting for a debug client to attach...")
        try:
            debugpy.wait_for_client()
        except Exception as exc:
            _log("wait for client failed: %s" % exc)
            return
        _log("debug client attached")


def _load_shadowed_sitecustomize():
    """Run any sitecustomize this module shadows (ours comes first on sys.path)."""
    import importlib.machinery
    import importlib.util

    here = os.path.dirname(os.path.abspath(__file__))
    others = [p for p in sys.path if p and os.path.abspath(p) != here]
    try:
        spec = importlib.machinery.PathFinder.find_spec("sitecustomize", others)
        if spec is None or spec.loader is None:
            return
        module = importlib.util.module_from_spec(spec)
        sys.modules["sitecustomize"] = module
        spec.loader.exec_module(module)
    except Exception as exc:
        _log("could not chain to the existing sitecustomize: %s" % exc)


try:
    _start_debugger()
except Exception as exc:  # never break the server because of the debugger
    _log("debugger disabled: %s" % exc)

_load_shadowed_sitecustomize()
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "odx-debug-{}-{}-{:?}",
            label,
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn prepare_writes_a_shim_that_python_can_import() {
        let root = temp_root("shim");
        let (setup, shim_dir) = prepare(&root, Some(5678), false).unwrap();

        let shim = shim_dir.join(SHIM_FILE);
        assert!(shim.exists(), "sitecustomize.py must exist for PYTHONPATH");
        assert_eq!(shim_dir, root.join(SHIM_DIR));
        assert_eq!(setup.host, HOST, "never bind a debug port off loopback");

        let source = fs::read_to_string(&shim).unwrap();
        // The two behaviours established by testing against debugpy: no patching of
        // os.exec* (Odoo restarts in place), and retrying the bind after a restart.
        assert!(source.contains("subProcess=False"));
        assert!(source.contains("def _listen"));
        assert!(source.contains("ODX_DEBUG_TARGET"));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn env_points_python_at_the_shim_and_routes_breakpoints() {
        let root = temp_root("env");
        let (setup, shim_dir) = prepare(&root, Some(5678), true).unwrap();
        let odoo_bin = root.join("src/odoo/odoo-bin");

        let env: std::collections::HashMap<String, String> =
            setup.env(&shim_dir, &odoo_bin).into_iter().collect();

        assert!(env["PYTHONPATH"].starts_with(&shim_dir.to_string_lossy().to_string()));
        assert_eq!(env["PYTHONBREAKPOINT"], "debugpy.breakpoint");
        assert_eq!(env["ODX_DEBUG_HOST"], HOST);
        assert_eq!(env["ODX_DEBUG_TARGET"], odoo_bin.to_string_lossy());
        assert_eq!(env["ODX_DEBUG_WAIT"], "1");
        assert_eq!(env["PYDEVD_DISABLE_FILE_VALIDATION"], "1");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn port_selection_skips_a_busy_port() {
        let taken = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let busy = taken.local_addr().unwrap().port();

        let chosen = pick_port(busy);

        assert_ne!(chosen, busy, "a second project must still get a debugger");
        assert!(chosen > busy && chosen <= busy + PORT_SCAN);
        assert!(port_is_free(chosen));
    }

    #[test]
    fn waiting_for_a_free_port_gives_up_instead_of_blocking_the_start() {
        let taken = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        let busy = taken.local_addr().unwrap().port();

        let started = Instant::now();
        let free = wait_for_port_free(busy, Duration::from_millis(200));

        assert!(!free);
        assert!(started.elapsed() < Duration::from_secs(2), "must not hang");
    }

    #[test]
    fn launch_json_attaches_to_the_chosen_port() {
        let setup = DebugSetup {
            host: HOST.to_string(),
            port: 5679,
            wait: false,
        };

        let json: serde_json::Value = serde_json::from_str(&launch_json(&setup)).unwrap();
        let cfg = &json["configurations"][0];

        assert_eq!(cfg["request"], "attach");
        assert_eq!(cfg["type"], "debugpy");
        assert_eq!(cfg["connect"]["host"], HOST);
        assert_eq!(cfg["connect"]["port"], 5679);
    }
}
