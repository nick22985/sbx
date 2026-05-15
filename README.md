# sbx

Sandboxed Docker dev environments. Pick a flavor (npm, bun, rust, claude, …),
`sbx init` a project, then `sbx shell` (or `sbx run`) to drop into a container
with your repo mounted at `/workspace`. Single static Rust binary; dynamic
shell completions via `clap_complete`.

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

`sbx completions <shell>` also prints a static completion script if you'd
rather check one in.

## Project lifecycle

```
sbx init [-p] <flavor>  Mark cwd as <flavor> and build the image
                        -p stores the marker in $SBX_PRIVATE_DIR instead of ./.sbx
sbx                     Print help (bare sbx is a no-op shortcut)
sbx shell               Enter the project's container
sbx <flavor>            Ad-hoc transient shell of <flavor> in cwd
sbx run                 Run `.sbx/start` in a fresh container
sbx sessions            List running sbx containers (alias: ps)
sbx stop                Stop containers, services, and network sidecars
sbx list                List available flavors
```

## Images

```
sbx build [flavor|all]    Rebuild image(s)
sbx rebuild [flavor|all]  Rebuild with --no-cache
sbx clean [flavor]        Remove cache volumes
sbx purge [flavor]        Remove caches + images (prompts)
sbx scan [fs|image]       Full trivy scan
```

## Per-project config

All per-project state lives under `sbx config` (aliases: `cfg`, `conf`).

```
sbx config port     [list|add N|rm N]
sbx config hostname [list|add HOST PORT|rm HOST]   Map HOST.sbx.localhost via the proxy sidecar
sbx config env      [list|set K=V|unset K]         Manages ~/.config/sbx/env
sbx config start    [show|set <cmd>|clear]
sbx config service  [list|add NAME|rm NAME]        Built-ins: redis, postgres, mongo, mysql, mailpit
sbx config ssh      [on|off|status]                Mount $SSH_AUTH_SOCK on next start
sbx config docker   [on|off|status]                Forward /var/run/docker.sock into the sandbox
```

## Networking

```
sbx net vpn       [status|use SPEC|auth|inline|off]
sbx net tailscale [on [name]|off|status|auth [name]|list|rm name]
sbx proxy         [status|routes|logs [-f]|stop]
```

`sbx proxy` controls the shared Traefik sidecar that publishes
`*.sbx.localhost` routes from `sbx config hostname` and from any container labels.
The Traefik dashboard is at http://traefik.sbx.localhost/dashboard/ whenever
the sidecar is up.

VPN/Tailscale settings are stored per-project in `.sbx/network` and applied
on the next `sbx` shell start. Tailscale supports multiple named profiles —
each maps to its own `SBX_TAILSCALE_AUTHKEY[_<NAME>]` env var.

## sbx claude

`sbx claude` is one of the flavors but has its own subtree because it has
some extra knobs:

```
sbx claude [-m PATH]... [-p PROFILE] [-s] [--no-rc] [--docker] [args...]
sbx claude shell [-m PATH]...     Drop to bash inside the sandbox
sbx claude build|rebuild          Build/rebuild the claude image
sbx claude profile [list|add NAME|rm NAME|current]
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

### Profiles

`sbx claude profile add work` creates an isolated `~/.claude` clone under
`$XDG_CONFIG_HOME/sbx/claude-profiles/work/` seeded from your host
`.claude.json`. Use it with `sbx claude -p work`, or pin a project to a
profile by writing the name into `./.sbx/claude-profile`. Useful for
separating personal/work logins or for keeping different MCP setups apart.

### Docker socket forwarding (opt-in)

`sbx config docker on` touches `./.sbx/docker`; every container start for that project
then bind-mounts `/var/run/docker.sock` from the host and `--group-add`s the
host docker GID so the unprivileged in-container user can talk to it. The base
image ships the docker client binary.

`sbx claude` intentionally does *not* follow `.sbx/docker` — opt in per-session
with `--docker`, or globally with `SBX_DOCKER=1` in `~/.config/sbx/env`.

**Security:** mounting the docker socket is effectively root on the host —
anything inside the container can `docker run --privileged -v /:/host …` and
escape the sandbox. Only enable this when you trust what's running inside.

## Files

- `$XDG_CONFIG_HOME/sbx/<flavor>/Dockerfile` — base image source
- `$XDG_CONFIG_HOME/sbx/env`                  — persistent env (KEY=value, chmod 600)
- `$XDG_CONFIG_HOME/sbx/claude-mounts`        — extra host paths for *every* `sbx claude` session
- `$XDG_CONFIG_HOME/sbx/claude-profiles/<n>/` — alternate `~/.claude` per profile
- `./.sbx/flavor`                             — per-project marker
- `./.sbx/Dockerfile`                         — optional, extends base
- `./.sbx/ports`                              — one port per line
- `./.sbx/hostname`                           — `host = port` or `host/path = port` lines, exposed via the proxy
- `./.sbx/start`                              — shell command for `sbx run`
- `./.sbx/network`                            — `vpn = …` / `tailscale = …` per-project network config
- `./.sbx/services`                           — sidecar service per line
- `./.sbx/ssh`                                — touched file → mount $SSH_AUTH_SOCK
- `./.sbx/docker`                             — touched file → mount /var/run/docker.sock
- `./.sbx/claude-mounts`                      — extra host paths for `sbx claude`, one per line
- `./.sbx/claude-profile`                     — pins this project to a named claude profile

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
| `SBX_PRIVATE_DIR=…` | Read-only overlay for `.sbx` configs; also where `sbx init -p` writes |
| `SBX_PROJECT_DIR=…` | Override the detected project root |
| `SBX_DOCKER=1` | Default `sbx claude` to mount the host docker socket |
| `SBX_REMOTE_CONTROL=0` | Disable `sbx claude` auto-`--remote-control` |
| `SBX_TAILSCALE_AUTHKEY[_<NAME>]=…` | Auth key for the default / named tailscale profile |
| `SBX_TAILSCALE_EXTRA_ARGS=…` | Extra args appended to `tailscale up` |
| `SBX_BUILDX_BUILDER=default` | Buildx builder to use for sbx's own builds (default: `default`; set empty to inherit `docker buildx use`) |

Persist these in `~/.config/sbx/env` (KEY=value lines, chmod 600). Host env
wins over the file.
