//! IPC server: lets the bar, settings app and `saforactl` talk to the compositor.

use std::{
    cell::RefCell,
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    rc::Rc,
};

use smithay::reexports::calloop::{generic::Generic, Interest, LoopHandle, Mode, PostAction};
use safora_common::ipc::{socket_path, Request};

use crate::state::SaforaState;

pub fn init(handle: &LoopHandle<'static, SaforaState>) -> std::io::Result<()> {
    let path = socket_path();
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(true)?;
    // The socket carries window-management commands: keep it private to the user.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;

    let inner = handle.clone();
    handle
        .insert_source(Generic::new(listener, Interest::READ, Mode::Level), move |_, listener, _state| {
            loop {
                match listener.accept() {
                    Ok((stream, _)) => accept_client(&inner, stream),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        tracing::warn!("ipc accept: {e}");
                        break;
                    }
                }
            }
            Ok(PostAction::Continue)
        })
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(())
}

fn accept_client(handle: &LoopHandle<'static, SaforaState>, stream: UnixStream) {
    if stream.set_nonblocking(true).is_err() {
        return;
    }
    let buf = Rc::new(RefCell::new(Vec::<u8>::new()));
    let _ = handle.insert_source(Generic::new(stream, Interest::READ, Mode::Level), move |_, stream, state| {
        let mut chunk = [0u8; 4096];
        let mut closed = false;
        loop {
            // Safety: we only read from the stream and never drop it here.
            match unsafe { stream.get_mut() }.read(&mut chunk) {
                Ok(0) => {
                    closed = true;
                    break;
                }
                Ok(n) => buf.borrow_mut().extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => {
                    closed = true;
                    break;
                }
            }
            if buf.borrow().len() > 64 * 1024 {
                closed = true; // refuse absurdly long lines
                break;
            }
        }

        // Process complete lines.
        loop {
            let line = {
                let mut b = buf.borrow_mut();
                match b.iter().position(|c| *c == b'\n') {
                    Some(pos) => b.drain(..=pos).collect::<Vec<u8>>(),
                    None => break,
                }
            };
            match serde_json::from_slice::<Request>(&line) {
                Ok(req) => {
                    let s = unsafe { stream.get_mut() };
                    handle_request(state, req, s);
                }
                Err(e) => tracing::debug!("ipc: bad request: {e}"),
            }
        }
        Ok(if closed { PostAction::Remove } else { PostAction::Continue })
    });
}

fn handle_request(state: &mut SaforaState, req: Request, stream: &mut UnixStream) {
    state.needs_render = true;
    match req {
        Request::Subscribe | Request::GetState => {
            let snapshot = state.ipc_state();
            let mut line = serde_json::to_string(&snapshot).unwrap_or_default();
            line.push('\n');
            let _ = stream.write_all(line.as_bytes());
            if matches!(req, Request::Subscribe) {
                if let Ok(clone) = stream.try_clone() {
                    state.ipc_subscribers.push(clone);
                }
            }
        }
        Request::Workspace { n } => state.switch_workspace(n.saturating_sub(1)),
        Request::MoveToWorkspace { n } => state.move_focused_to_workspace(n.saturating_sub(1)),
        Request::Focus { id } => {
            if let Some(ws) = state.client_by_id(id).map(|c| c.workspace) {
                state.switch_workspace(ws);
                // Picked from the bar's window list: the window gets the keyboard, not the bar.
                if state.layer_releasable() {
                    state.layer_focus = None;
                }
                state.focus_client(Some(id));
            }
        }
        Request::FocusNext => state.focus_cycle(true),
        Request::FocusPrev => state.focus_cycle(false),
        Request::CloseFocused => state.close_focused(),
        Request::Close { id } => state.close_client(id),
        Request::ToggleFloating => state.toggle_floating(),
        Request::ToggleFullscreen => state.toggle_fullscreen(),
        Request::CycleLayout => state.cycle_layout(),
        Request::Swap { forward } => state.swap_focused(forward),
        Request::Spawn { command } => state.spawn(&command),
        Request::GetOutputs => {
            let mut line = serde_json::to_string(&state.outputs_info()).unwrap_or_default();
            line.push('\n');
            let _ = stream.write_all(line.as_bytes());
        }
        Request::Reload => state.reload_config(),
        Request::Lock => state.lock(),
        Request::Unlock { token } => state.unlock(&token),
        Request::Screenshot { path } => state.screenshot_request = Some(path),
        Request::Quit => state.loop_signal.stop(),
    }
}
