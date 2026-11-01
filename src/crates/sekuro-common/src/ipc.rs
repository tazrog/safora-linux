//! Newline-delimited JSON protocol spoken over `$XDG_RUNTIME_DIR/sekuro.sock`.
//!
//! A client may send [`Request`]s at any time. After a `Subscribe` request the
//! compositor streams a [`State`] line whenever anything changes.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub fn socket_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
    dir.join("sekuro.sock")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// Send the current state once and keep sending updates.
    Subscribe,
    /// Send the current state once and close.
    GetState,
    Workspace { n: usize },
    MoveToWorkspace { n: usize },
    Focus { id: u32 },
    FocusNext,
    FocusPrev,
    CloseFocused,
    Close { id: u32 },
    ToggleFloating,
    ToggleFullscreen,
    CycleLayout,
    Swap { forward: bool },
    Spawn { command: String },
    /// Describe the outputs and their available modes.
    GetOutputs,
    /// Re-read `config.toml`.
    Reload,
    /// Save a PNG of the output (nested/development backend only).
    Screenshot { path: String },
    Quit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkspaceInfo {
    pub id: usize,
    pub windows: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_id: String,
    pub workspace: usize,
    pub focused: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct State {
    pub workspaces: Vec<WorkspaceInfo>,
    pub active: usize,
    pub windows: Vec<WindowInfo>,
    pub layout: String,
}

/// Send one request to the running compositor (fire and forget).
pub fn send(req: &Request) -> std::io::Result<()> {
    use std::io::Write;
    let mut s = std::os::unix::net::UnixStream::connect(socket_path())?;
    let mut line = serde_json::to_string(req).map_err(std::io::Error::other)?;
    line.push('\n');
    s.write_all(line.as_bytes())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModeInfo {
    pub width: u32,
    pub height: u32,
    /// Refresh rate in millihertz.
    pub refresh_mhz: u32,
    pub preferred: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OutputInfo {
    pub name: String,
    pub current: Option<ModeInfo>,
    pub modes: Vec<ModeInfo>,
}

/// Ask the compositor for its outputs. Returns an empty list if it isn't running.
pub fn get_outputs() -> Vec<OutputInfo> {
    use std::io::{BufRead, BufReader, Write};
    let Ok(mut s) = std::os::unix::net::UnixStream::connect(socket_path()) else { return vec![] };
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(2)));
    let mut line = serde_json::to_string(&Request::GetOutputs).unwrap_or_default();
    line.push('\n');
    if s.write_all(line.as_bytes()).is_err() {
        return vec![];
    }
    let mut reply = String::new();
    if BufReader::new(s).read_line(&mut reply).is_err() {
        return vec![];
    }
    serde_json::from_str(&reply).unwrap_or_default()
}
