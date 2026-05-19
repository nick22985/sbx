# Example `~/.config/sbx/`

These files mirror the layout of `$XDG_CONFIG_HOME/sbx/`
(`~/.config/sbx/` on most setups). The flavor Dockerfiles are working
definitions pulled from my dotfiles, dropped in as a reference for how
to author your own.

sbx loads flavors from `$XDG_CONFIG_HOME/sbx/flavors/<flavor>/Dockerfile`.
To try one:

```sh
mkdir -p ~/.config/sbx/flavors
cp -r examples/config/flavors/base ~/.config/sbx/flavors/
cp -r examples/config/flavors/npm  ~/.config/sbx/flavors/   # plus whichever others
sbx build base       # build the shared base first
sbx build npm        # then the leaf flavor

cd ~/path/to/some/project
sbx init npm         # mark the project + build if needed
sbx shell            # drop into the sandbox
```

Optional: copy `config.toml` to `~/.config/sbx/config.toml` for global
`mounts` (e.g. `~/.m2`, `~/.gradle`) and `caches` (host binds for
npm/cargo/pip/...) applied to every sbx session.

Coming from an older layout where flavors sat at the top level of
`~/.config/sbx/`? Run `sbx migrate` once; it relocates each
`<flavor>/Dockerfile` into `flavors/<flavor>/` and consolidates any
legacy `caches` files into per-flavor `config.toml`.

## Layout

```
config.toml           Example global config (mounts + caches applied to every session).
flavors/base/         Shared layer every other flavor FROMs. Debian slim +
                      git/curl/python + osv-scanner + trivy + guarddog +
                      mise + docker client + `sbx-audit`. Build first.
flavors/npm/          Node via mise. Bundles socket.dev CLI as `socket`.
flavors/bun/          Bun + Node via mise. Bundles socket.dev CLI as `socket`.
flavors/rust/         rustup-managed toolchain + cargo-audit (`cargo audit`).
flavors/java/         Java + Maven via mise.
flavors/claude/       Heavyweight image with node + bun + rust + java + python +
                      the Claude Code CLI + graphify. Used by `sbx claude`. Its
                      entrypoint runs `graphify install` once into ~/.claude.
flavors/<flavor>/config.toml
                      Per-flavor config (caches shipped with the flavor's Dockerfile).
```

## What's in `base/`

A few things are bundled into `base/` so every flavor inherits them
instead of repeating them:

- **Scanners** — `osv-scanner`, `trivy`, `guarddog` (the last gated by
  `--build-arg SBX_GUARDDOG=0` if you want it skipped).
- **`sbx-audit`** — small wrapper at `/opt/sbx/bin/sbx-audit` that runs
  whatever scanners are installed against the current directory. Run it
  explicitly when you want to check a project; nothing in sbx invokes
  it for you.
- **mise** — installed to `/usr/local/bin/mise`, used by every leaf
  flavor to manage tool versions. Project `mise.toml` / `.tool-versions`
  files are honored automatically when mounted at `/workspace`.
- **docker client** — static binary, so `sbx config docker on` can
  forward the host socket without needing to install docker per-flavor.
- **`dev` user** — UID/GID rewritten to match the host's at build time
  (`USER_UID` / `USER_GID` build args, defaulted to 1000) so files
  written from the container land on the host with sensible ownership.

## Auditing inside the sandbox

Tools like `npm`, `bun`, `cargo` and `mvn` run unmodified — what you
type is what runs. To audit, invoke a scanner directly:

```sh
sbx-audit              # osv-scanner --recursive .  +  trivy fs .
sbx-audit --osv        # one tool only
osv-scanner --recursive .
trivy fs .
socket scan create .   # npm/bun flavors — socket.dev policy + reputation
guarddog npm verify lodash   # heuristics on a single package
cargo audit            # rust flavor only — Cargo.lock vuln scan
```

`sbx-audit` is the umbrella; the individual binaries are all on `PATH`
if you want to script around them.

## Modifying

These are starting points. Edit the copy in
`~/.config/sbx/flavors/<flavor>/` freely; `sbx build <flavor>` will pick
up the changes. The `examples/config/` tree in this repo is just static
reference.
