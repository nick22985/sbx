# sbx

Sandboxed Docker dev environments for **npm / bun / rust**, plus a
top-level **`sbx claude`** subcommand that runs Claude Code in a sandbox
on top of any project.

A Rust rewrite of the original bash `sbx` script. Single binary, dynamic shell
completions via `clap_complete`.

## Install

```sh
./install.sh
```

Then for completions, add to your shell rc:

```sh
# Bash
source <(COMPLETE=bash sbx)
# Zsh
source <(COMPLETE=zsh sbx)
# Fish
COMPLETE=fish sbx | source
```

## Usage

```
sbx init <flavor>       Mark cwd as <flavor> and build the image
sbx                     Enter the container (alias of `sbx shell`)
sbx <flavor>            Ad-hoc shell of <flavor> in cwd
sbx run                 Run `.sbx/start` in a fresh container
sbx stop                Stop containers, services, and VPN sidecar
sbx build [flavor]      Rebuild image
sbx rebuild [flavor]    Rebuild image with --no-cache
sbx clean [flavor]      Remove cache volumes
sbx purge [flavor]      Remove caches + images (prompts)
sbx list                List flavors

sbx port [list|add N|rm N]
sbx env  [list|set K=V|unset K]   Manages ~/.config/sbx/env
sbx start [show|set <cmd>|clear]
sbx scan [fs|image]               Full trivy scan
sbx service [list|add NAME|rm NAME]
sbx ssh [on|off|status]           Mount $SSH_AUTH_SOCK on next start
sbx vpn [status|use SPEC|auth|inline|off]

sbx claude [-m PATH]... [-s] [args...] Launch Claude Code in a sandbox over cwd
sbx claude shell [-m PATH]...     Drop to bash inside the sandbox
sbx claude build|rebuild          Build/rebuild the claude image
```

`sbx claude` is independent of the project's flavor — you can launch it
on an npm/bun/rust/uninitialised project. It mounts cwd at `/workspace`
and the host's `~/.claude` rw (so auth, config, and history are shared).
The image bundles node + bun + rust + python and the Claude Code CLI.

Because the container is already a sandbox, `sbx claude` auto-passes
`--dangerously-skip-permissions` to `claude` so prompts don't get in the
way. Pass `-s` / `--safe` to opt out for a single invocation, or pass
`--dangerously-skip-permissions` yourself and it won't be duplicated.

Extra host paths can be made visible inside the sandbox in three ways
(all opt-in, off by default):

- `-m / --mount <PATH>` — repeatable, ad-hoc per invocation
  (`sbx claude -m ~/projects/foo -m ~/projects/bar`)
- `./.sbx/claude-mounts` — one absolute path per line, `#` comments
  allowed. Persistent for projects that always want the same companions.
- `$XDG_CONFIG_HOME/sbx/claude-mounts` — same format, applied to **every**
  `sbx claude` session. Good for caches/tooling you always want (e.g.
  `~/.m2`, `~/.gradle`, `~/.cache/pip`).

Each path is mounted at its host absolute path inside the container, so
Claude Code sees the same paths inside and out.

By default `sbx claude` opts each session into [Remote Control] so it's
reachable from `claude.ai/code` and the Claude mobile app — sbx appends
`--remote-control "<project>-<pid>"` to the inner `claude` invocation.
Opt out with `--no-rc` for a single run, `SBX_REMOTE_CONTROL=0` to
disable persistently, or by passing your own `--remote-control` / `--rc`
flag (sbx won't double up).

[Remote Control]: https://code.claude.com/docs/en/remote-control

## Files

- `$XDG_CONFIG_HOME/sbx/<flavor>/Dockerfile` — base image source
- `$XDG_CONFIG_HOME/sbx/claude-mounts`        — extra host paths for *every* `sbx claude` session
- `./.sbx/flavor`                            — per-project marker
- `./.sbx/Dockerfile`                        — optional, extends base
- `./.sbx/ports`                             — one port per line
- `./.sbx/start`                             — shell command for `sbx run`
- `./.sbx/vpn`                               — `.ovpn` path or name under `$SBX_VPN_DIR`
- `./.sbx/services`                          — sidecar service per line
- `./.sbx/ssh`                               — touched file → mount $SSH_AUTH_SOCK
- `./.sbx/claude-mounts`                     — extra host paths for `sbx claude`, one per line

In a git worktree, `.sbx/*` files are looked up in the worktree first, then
the shared bare/primary repo, then the private overlay
(`$SBX_PRIVATE_DIR/<rel-path>/.sbx/`).

## Environment

| Var | Meaning |
|-----|---------|
| `SBX_PORTS=3000,8080` | Extra ports to publish |
| `SBX_RUN_SCRIPTS=1` | Allow npm/bun postinstall scripts |
| `SBX_SCANNERS=osv,socket` | Scanner allowlist (or `none`) |
| `SOCKET_CLI_API_TOKEN=…` | socket.dev API token |
| `SOCKET_ORG_SLUG=…` | Default org for `socket scan create` |
| `SBX_VPN_DIR=…` | Directory for bare VPN names |
| `SBX_PRIVATE_DIR=…` | Read-only overlay for `.sbx` configs |

Persist these in `~/.config/sbx/env` (KEY=value lines, chmod 600). Host env
wins over the file.
