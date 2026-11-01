//! sekuroctl — control a running sekurowm from the shell or scripts.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use sekuro_common::ipc::{self, Request};

const USAGE: &str = "usage: sekuroctl <command>

  state                 print workspaces and windows as JSON
  workspace N           switch to workspace N (1-based)
  move N                move the focused window to workspace N
  focus-next|focus-prev cycle focus
  close                 close the focused window
  float                 toggle floating for the focused window
  fullscreen            toggle fullscreen
  layout                cycle dwindle / master / columns
  swap-next|swap-prev   reorder the focused tile
  spawn COMMAND...      run a command inside the session
  outputs               list monitors and their display modes
  reload                re-read ~/.config/sekuro/config.toml
  screenshot FILE.png   save the screen (nested/dev mode only)
  quit                  end the session";

fn parse(args: &[String]) -> Result<Request, String> {
    let n = |i: usize| -> Result<usize, String> {
        args.get(i).ok_or("missing number")?.parse().map_err(|_| "not a number".to_string())
    };
    Ok(match args.first().map(String::as_str) {
        Some("state") => Request::GetState,
        Some("workspace") => Request::Workspace { n: n(1)? },
        Some("move") => Request::MoveToWorkspace { n: n(1)? },
        Some("focus-next") => Request::FocusNext,
        Some("focus-prev") => Request::FocusPrev,
        Some("close") => Request::CloseFocused,
        Some("float") => Request::ToggleFloating,
        Some("fullscreen") => Request::ToggleFullscreen,
        Some("layout") => Request::CycleLayout,
        Some("swap-next") => Request::Swap { forward: true },
        Some("swap-prev") => Request::Swap { forward: false },
        Some("spawn") if args.len() > 1 => Request::Spawn { command: args[1..].join(" ") },
        Some("outputs") => Request::GetOutputs,
        Some("reload") => Request::Reload,
        Some("screenshot") if args.len() > 1 => Request::Screenshot { path: args[1].clone() },
        Some("quit") => Request::Quit,
        _ => return Err(USAGE.into()),
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let req = match parse(&args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let mut stream = match UnixStream::connect(ipc::socket_path()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("sekuroctl: cannot reach sekurowm ({e}) — is it running?");
            std::process::exit(1);
        }
    };
    let mut line = serde_json::to_string(&req).unwrap();
    line.push('\n');
    stream.write_all(line.as_bytes()).unwrap();
    if matches!(req, Request::GetState | Request::GetOutputs) {
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).ok();
        print!("{reply}");
    }
}
