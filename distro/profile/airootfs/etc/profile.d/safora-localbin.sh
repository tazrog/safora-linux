# Per-user programs (e.g. Claude Code from its native installer) live in ~/.local/bin.
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) PATH="$HOME/.local/bin:$PATH" ;; esac
export PATH
