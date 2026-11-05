//! saforactl — control a running saforawm from the shell or scripts.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use safora_common::ipc::{self, Request};

const USAGE: &str = "usage: saforactl <command>

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
  lock                  lock the screen
  reload                re-read ~/.config/safora/config.toml
  screenshot FILE.png   save the screen (nested/dev mode only)
  quit                  end the session

  claude-status         Claude Code status-line command: reads Claude Code's JSON on stdin and
                        keeps its 5-hour / weekly usage for the bar's Claude widget
  claude-connect [--replace]   make Claude Code report its usage to the bar
  claude-disconnect     undo claude-connect";

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
        Some("lock") => Request::Lock,
        Some("screenshot") if args.len() > 1 => Request::Screenshot { path: args[1].clone() },
        Some("quit") => Request::Quit,
        _ => return Err(USAGE.into()),
    })
}

/// `saforactl claude-*`: talks to Claude Code's files, not to the compositor.
fn claude(args: &[String]) -> i32 {
    use safora_common::claude;
    match args[0].as_str() {
        "claude-status" => {
            let mut input = String::new();
            let _ = std::io::Read::read_to_string(&mut std::io::stdin().lock(), &mut input);
            let v: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();
            let now = claude::now();
            let usage = claude::from_statusline(&v, now);
            // Keep the last good numbers if this update carried none (e.g. before the first reply).
            let shown = match claude::load() {
                Some(old) if !usage.available && old.available => claude::Usage { model: usage.model.clone(), ..old },
                _ => {
                    if let Err(e) = claude::save(&usage) {
                        eprintln!("saforactl: cannot store Claude usage: {e}");
                    }
                    usage
                }
            };
            println!("{}", claude::status_text(&shown, now));
            0
        }
        "claude-connect" => match claude::connect(args.get(1).is_some_and(|a| a == "--replace")) {
            Ok(()) => {
                println!("Claude Code now reports its usage to the Safora bar ({}).", claude::settings_path().display());
                0
            }
            Err(e) => {
                eprintln!("saforactl: {e}");
                1
            }
        },
        _ => match claude::disconnect() {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("saforactl: {e}");
                1
            }
        },
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("claude-status" | "claude-connect" | "claude-disconnect")) {
        std::process::exit(claude(&args));
    }
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
            eprintln!("saforactl: cannot reach saforawm ({e}) — is it running?");
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
