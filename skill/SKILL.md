---
name: s2-engine
description: Run local language models on this machine with the s2 command. Use when asked to list, start, stop, switch, check, add, download or remove a local model or profile, or when a local model endpoint on 127.0.0.1 is down.
---

# S2 Engine

`s2` (also `s2-engine`) runs one local model server at a time, because they share one GPU. A **profile** is a runnable entry: a model file, a runtime (a named `llama-server` build) and settings. `s2 --help` and `s2 <command> --help` list the flags.

## What needs the user's request

| Do freely (read only) | Do when the user asks | Do only when the user names the exact target |
|---|---|---|
| `s2 list`, `s2 status`, `s2 show <name>`, `s2 pull --check`, `s2 start <name> --dry-run` | `s2 start`, `s2 stop`, `s2 add`, `s2 pull` | `s2 remove <name>`, `--delete-file`, changing the config file |

## Switching models can cut you off

The running model may be the one serving you: hermes' default model is a local profile. Before `start` or `stop`:

1. Run `s2 status`.
2. If a model is running, tell the user which one will stop, and that you may lose your own model if you run on it. Wait for their yes.
3. Then run the command with `--force`. Outside a terminal `s2` refuses to stop a running model without it.

`s2 start` blocks until the server answers, up to 240 seconds. On failure it prints the end of `~/llama-server.log`; read that file to diagnose.

Starting a profile with `"hermes": true` rewrites hermes' default model and context length; hermes sessions need a restart to pick it up.

## Adding a profile

There is no terminal to prompt you, so pass every required value:

```
s2 add <name> --runtime <runtime> --context <tokens> --port <port> \
  (--model <file> | --repo <hf-repo> --file <file-in-repo>) -- <extra server flags>
```

Runtime names are the keys under `runtimes` in the config (`~/.config/s2-engine/models.json`). With `--repo`, `add` downloads the file, often many gigabytes; `--no-download` records it only. Check free disk space and VRAM first.

Downloading never changes which model is running. Deleting a model file outside a terminal also needs `--yes`.
