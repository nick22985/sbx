# Project `.sbx/` examples

Every sbx project keeps its config in a `.sbx/` directory at the project
root. This folder contains one annotated example per file sbx looks for,
so you can see all the knobs in one place.

Copy whichever pieces you need into your project's `.sbx/`:

```sh
cd ~/path/to/some/project
sbx init npm                 # creates .sbx/flavor
cp ../sbx/examples/sbx/ports         .sbx/ports
cp ../sbx/examples/sbx/services      .sbx/services
cp ../sbx/examples/sbx/hostname      .sbx/hostname
# ... etc
```

## Files

| File              | Purpose                                                        | Format                                |
| ----------------- | -------------------------------------------------------------- | ------------------------------------- |
| `flavor`          | Required. Names the flavor (`npm`, `rust`, ...).               | One line, the flavor name.            |
| `Dockerfile`      | Optional. Per-project layer on top of the flavor image.        | Standard Dockerfile.                  |
| `start`           | Command for `sbx run`.                                         | Shell, passed to `bash -c`.           |
| `ports`           | Ports forwarded to `127.0.0.1`.                                | One u16 per line, `#` comments ok.    |
| `services`        | Sidecar services (postgres, redis, ...).                       | One spec per line, `#` comments ok.   |
| `hostname`        | Traefik routes for `sbx proxy`.                                | `host[/path] = port` per line.        |
| `network`         | VPN / Tailscale per-project policy.                            | `key=value` per line.                 |
| `ssh`             | Marker. Enables SSH agent + `~/.ssh/{config,known_hosts}` mount. | Existence-only; content ignored.    |
| `docker`          | Marker. Forwards the host docker socket. **Trusted projects only.** | Existence-only; content ignored. |
| `host-proxy`      | Marker + optional allowlist for the host tinyproxy sidecar.    | One hostname per line, `#` comments. Empty = unrestricted. |
| `mounts`          | Extra host mounts for every sbx session.                       | `host[:container[:ro]]` per line, `#` comments ok. |
| `caches`          | Per-project cache mounts, layered on flavor + global caches.   | `host_rel[:container]` or `@volume:container` per line. |
| `claude-profile`  | Pins a named claude profile for this project.                  | One line, the profile name.           |

`flavor` is the only required file — everything else is opt-in.

## Generated vs hand-edited

`.sbx/` is meant to be hand-edited and committed. Some commands write into
a private mirror dir under `$XDG_DATA_HOME/sbx/private/<key>/.sbx/` so they
don't churn the repo — `sbx config ports add 3000` is one example.
Hand-edits to `.sbx/ports` always take precedence when sbx reads.
