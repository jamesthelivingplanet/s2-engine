# S2 Engine

Run local language models, one at a time. Installs two names for the same command: `s2` and `s2-engine`. See `GLOSSARY.md` for terms.

## Install

```sh
cargo install --path . --root ~/.local
```

Needs `llama-server` builds you provide (the tool does not build or download them), and the Hugging Face `hf` command for downloads. Linux only.

## Commands

| Command | What it does |
|---|---|
| `s2`, `s2 list` | Profiles, whether their model file is present, and which one is running |
| `s2 start [profile]` | Start by name or list number, replacing what is running. `--dry-run` prints the server command |
| `s2 stop`, `s2 status` | |
| `s2 talk <profile> [system prompt]` | Terminal chat; `/reset` clears history, Ctrl-C cancels a reply, Ctrl-D exits |
| `s2 add <name>` | Prompts for missing values, or takes flags; downloads when given `--repo` and `--file` |
| `s2 remove <name>` | Removes the profile. `--delete-file` also deletes the model if nothing else uses it |
| `s2 show <name>`, `s2 edit` | Print one profile; edit the config with validation |
| `s2 pull [name]` | Download missing models. `--check` only reports, exit 1 if something can be downloaded |
| `s2 tray setup`, `s2 tray remove` | Tray icon started at login. `s2 tray` runs it in the foreground |
| `s2 skill install`, `s2 skill remove` | Agent skill for Claude, Codex and Hermes |

Outside a terminal, `start` and `stop` refuse to stop a running model without `--force`, so an agent cannot cut off its own model by accident. The tray menu is not restricted.

## Config

`~/.config/s2-engine/models.json`; see `examples/models.example.json`. It may be a symlink (into a dotfiles repo, say): saves go to the target.

- `runtimes`: name to `llama-server` path or command.
- Each profile: `name`, `description`, `runtime`, `model`, `alias`, `context`, `port`, `args` (one flag or value per element), `hermes`, and an optional `source` (`repo`, `file`, `size`).
- A leading `~/` is expanded in paths and args. With a `source`, `model` must end with `source.file`.
- `"hermes": true` rewrites `default` and `context_length` in the `model:` block of `~/.hermes/config.yaml` when the profile starts. Nothing else in that file is touched.

The server runs as `<runtime> -m <model> -c <context> <args> --alias <alias> --host 127.0.0.1 --port <port>`, with output in `~/llama-server.log` and its pid in `~/llama-server.pid`.

On Omarchy, tray icons sit behind the bar's `<` arrow. To keep this one visible, add `"pinned": ["s2-engine"]` to the `omarchy.tray` entry in `~/.config/omarchy/shell.json`.

## Environment overrides

`S2_CONFIG`, `S2_PIDFILE`, `S2_LOG`, `S2_STATE_DIR`, `S2_HERMES_CONFIG`, `S2_HF`, `S2_SYSTEMCTL`, `S2_START_TIMEOUT` (240 s), `S2_STOP_TIMEOUT` (30 s).

## Development

```sh
cargo test        # uses a fake server; needs python3
cargo clippy --all-targets
python3 assets/icons/render.py   # after changing the icon; needs rsvg-convert
```

The tray icon is an original drawing, not official Evangelion artwork.
