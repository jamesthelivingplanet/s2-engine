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
- Each profile: `name`, `description`, `runtime`, `model`, `alias`, `context`, `port`, `args` (one flag or value per element), `hermes`, an optional `source` (`repo`, `file`, `size`) and optional `hooks` (below).
- A leading `~/` is expanded in paths and args. With a `source`, `model` must end with `source.file`.
- `"hermes": true` rewrites `default` and `context_length` in the `model:` block of `~/.hermes/config.yaml` when the profile starts. Nothing else in that file is touched.

The server runs as `<runtime> -m <model> -c <context> <args> --alias <alias> --host 127.0.0.1 --port <port>`, with output in `~/llama-server.log` and its pid in `~/llama-server.pid`.

On Omarchy, tray icons sit behind the bar's `<` arrow. To keep this one visible, add `"pinned": ["s2-engine"]` to the `omarchy.tray` entry in `~/.config/omarchy/shell.json`.

## Hooks

A profile may run your own shell commands around the model:

```json
"hooks": {
  "pre": "systemctl stop display-manager",
  "post": "systemctl start display-manager",
  "sudo": true,
  "closes_session": true
}
```

| Key | Meaning |
|---|---|
| `pre` | Runs before the model starts. If it fails, the model does not start |
| `post` | Runs after the model ends: stop, switch, crash, failed start or failed `pre` |
| `sudo` | Both commands run as root (the model still runs as you). The password is asked at every start, not at stop |
| `closes_session` | `s2` asks you to type `yes` first |

`s2 start` shows the commands before running them. Profiles with `sudo` or `closes_session` start only from a terminal: the tray lists them as "terminal only", and `talk` does not auto-start any hooked profile. Write `pre` so that it returns only once the machine is ready, for example waiting until the desktop's processes have exited.

A hooked profile runs in a transient systemd unit (`s2-hooked`), which outlives your terminal and session. `s2 stop` ends the model and waits for `post`. A hooked start that does not become healthy in time is stopped, so that `post` runs. Logs: `journalctl -u s2-hooked` for `sudo` hooks, `journalctl --user -u s2-hooked` otherwise. Without your login session, `sudo` hooks need nothing more; other hooks need `loginctl enable-linger`.

**Anyone who can edit your config can put a command there that you will be asked to run as root. Read what `s2` shows before typing your password.**

## Environment overrides

`S2_CONFIG`, `S2_PIDFILE`, `S2_LOG`, `S2_STATE_DIR`, `S2_HERMES_CONFIG`, `S2_HF`, `S2_SYSTEMCTL`, `S2_SYSTEMD_RUN`, `S2_UNIT`, `S2_START_TIMEOUT` (240 s), `S2_STOP_TIMEOUT` (30 s), `S2_HOOK_TIMEOUT` (180 s).

## Development

```sh
cargo test        # uses a fake server; needs python3. Hook tests use your user systemd and `script`
cargo clippy --all-targets
python3 assets/icons/render.py   # after changing the icon; needs rsvg-convert
```

The tray icon is an original drawing, not official Evangelion artwork.

## License

MIT, see `LICENSE`.
