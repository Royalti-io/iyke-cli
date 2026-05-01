//! control.json discovery + stale-file handling.
//!
//! The desktop app writes `control.json` to its `app_local_data_dir`,
//! which Tauri computes from the bundle identifier. We mirror that path
//! here using `dirs::data_local_dir()` joined with the same identifier
//! constant — that way the CLI doesn't need a Tauri runtime to find it.
//!
//! Stale handling: if the recorded PID is no longer alive, the file is
//! a leftover from a previous run that exited via SIGKILL or similar.
//! Per design decision (paranoid auto-delete): only delete it if the
//! `started_at_unix_ms` timestamp is older than `STALE_THRESHOLD_SECS`,
//! to avoid racing with an app that's just starting up.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

pub const APP_IDENTIFIER: &str = "io.royalti.pa.desktop";

/// How long a control.json must have existed before we'll auto-delete it
/// when the PID is dead. 5 minutes is well past any normal app launch
/// race window but short enough that users don't have to wait around
/// after a crash.
pub const STALE_THRESHOLD_SECS: u64 = 5 * 60;

#[derive(Debug, Deserialize)]
pub struct ControlFile {
    pub schema_version: u32,
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub started_at_unix_ms: u128,
    #[allow(dead_code)]
    pub identifier: String,
}

/// Path the desktop app writes its control file to. Same computation as
/// the Tauri-side `app.path().app_local_data_dir().join("control.json")`.
pub fn control_path() -> Result<PathBuf> {
    let base = dirs::data_local_dir()
        .ok_or_else(|| anyhow!("could not determine local data dir for current user"))?;
    Ok(base.join(APP_IDENTIFIER).join("control.json"))
}

/// Outcome of trying to load and validate the control file.
pub enum LoadOutcome {
    Ok(ControlFile),
    /// File doesn't exist at all — app has never run, or shut down cleanly.
    Missing,
    /// File exists but the PID is dead AND it's been older than the
    /// stale threshold — we've already removed it; tell the user.
    StaleRemoved,
    /// File exists, PID is dead, but it's too young to safely remove —
    /// most likely a launch race. Tell the user to retry shortly.
    StaleYoung { age_secs: u64 },
}

pub fn load() -> Result<LoadOutcome> {
    let path = control_path()?;
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(LoadOutcome::Missing),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };

    let cf: ControlFile = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse {} as JSON", path.display()))?;

    if cf.schema_version != 1 {
        return Err(anyhow!(
            "unsupported control.json schema_version: {} (CLI built for v1)",
            cf.schema_version
        ));
    }

    if is_pid_alive(cf.pid) {
        return Ok(LoadOutcome::Ok(cf));
    }

    // PID is dead. Decide whether to clean up based on age.
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let age_ms = now_ms.saturating_sub(cf.started_at_unix_ms);
    let age_secs = (age_ms / 1000) as u64;

    if age_secs >= STALE_THRESHOLD_SECS {
        // Best effort — if removal fails, the user can clean up by hand
        // and the next launch overwrites it anyway.
        let _ = std::fs::remove_file(&path);
        Ok(LoadOutcome::StaleRemoved)
    } else {
        Ok(LoadOutcome::StaleYoung { age_secs })
    }
}

#[cfg(unix)]
fn is_pid_alive(pid: u32) -> bool {
    // kill(pid, 0) tests for process existence without delivering a signal.
    // Returns 0 on success (process exists). On failure: ESRCH = no such
    // process; EPERM = process exists but we can't signal it (still alive).
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    matches!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EPERM)
    )
}

#[cfg(not(unix))]
fn is_pid_alive(_pid: u32) -> bool {
    // Windows isn't a v1 target; assume alive so we don't false-positive
    // on stale handling. The caller can still get a connection error if
    // the server actually isn't there.
    true
}
