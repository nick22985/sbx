use std::path::Path;

use crate::config::{CONFIG_FILENAME, Config};
use crate::docker;
use crate::flavor::{build_image, image_name, is_flavor, is_internal_flavor, list_flavors};
use crate::project::{private_write_dir, sbx_write_dir};
use crate::util::{die, log};

const BLANK_TEMPLATE: &str = r#"# sbx project config. Edit directly or use `sbx config <subcommand>`.
# Layered with $SBX_PRIVATE_DIR + git common dir + local: scalars override
# (last wins), lists concat+dedupe, bools OR, maps merge.

flavor = "{FLAVOR}"

# Optional. Overrides the worktree suffix in the project name (and hostname
# prefix). Defaults to the branch name.
# name = ""

# Optional. Overrides the hash-derived per-worktree port shift. Set to 0 to
# pin a worktree to the base ports.
# port-offset = 0

# Ports forwarded to 127.0.0.1 on the host. Also flow through to sidecar
# services that share the netns.
# ports = []

# Shell command for `sbx run`. Passed to bash -c.
# start = ""

# Extra host paths mounted into every sbx session for this project. Layered
# on top of `mounts` in ~/.config/sbx/config.toml.
#
# Syntax (missing host paths are skipped):
#   "host"                     same path on both sides
#   "host:container"           explicit container path
#   "host:container:ro"        read-only
#   "host::ro"                 same-path bind, read-only
# mounts = []

# Extra cache mounts. Layered on the flavor's + global caches; entries with
# the same container path override earlier layers.
#
# Syntax:
#   "<host_rel>"                          host bind, container = ~/<host_rel>
#   "<host_rel>:<container_path>"         host bind, explicit container path
#   "@<volume_name>:<container_path>"     named docker volume
# caches = []

# SSH agent + ~/.ssh/{config,known_hosts} forwarding. Toggle with `sbx config ssh on/off`.
# ssh = false

# Forward the host docker socket. WARNING: anything in the container can
# break out to root on the host via this. Toggle with `sbx config docker on/off`.
# docker = false

# Wayland + X11 sockets forwarded so GUI apps render on the host. Toggle
# with `sbx config gui on/off`.
# gui = false

# [claude]
# profile = ""

# [network]
# vpn = ""              # wireguard config under $SBX_VPN_DIR
# tailscale = ""        # "1" for default profile, or a named profile

# Traefik routes for *.sbx.localhost via the proxy sidecar. Keys are
# hostnames (optionally with a path suffix); values are container ports.
# [hostname]
# "app.sbx.localhost" = 3000

# Public routes through the cloudflare tunnel sidecar.
# [public]
# "app.example.com" = 8080

# Sidecar services started alongside the project container. Share the
# project's netns, so `localhost:5432` hits postgres. Built-ins: redis,
# postgres, mongo, mysql, mailpit. Anything with `/` or `:` is a raw image.
# [services]
# enabled = []

# Per-project allowlist for the host tinyproxy sidecar. Empty `allow` with
# `enabled = true` is unrestricted.
# [host_proxy]
# enabled = false
# allow = []

# Port-forwarding tunnels for the socat sidecar.
#   dir = "out"        sandbox -> host:                  right = host port
#   dir = "in"         in-netns listener -> host:        right = host port
#   dir = "via"        host -> remote via sandbox netns: right = "host:port"
#   dir = "via-host"   sandbox -> remote via host netns: right = "host:port"
# [[tunnel]]
# dir = "out"
# left = 3000
# right = 3000
"#;

pub fn run(cwd: &Path, flavor: &str, private: bool) {
    if is_internal_flavor(flavor) {
        die(format!(
            "'{flavor}' isn't a project flavor - use `sbx {flavor}` to launch it directly"
        ));
    }
    if !is_flavor(flavor) {
        die(format!(
            "unknown flavor: {flavor} (have: {})",
            list_flavors().join(",")
        ));
    }
    let write_dir = if private {
        private_write_dir(cwd)
    } else {
        sbx_write_dir(cwd)
    };

    let path = if Config::exists_for(cwd) {
        let mut cfg = Config::load_or_default(cwd);
        cfg.flavor = Some(flavor.to_string());
        cfg.save_to_dir(&write_dir)
            .unwrap_or_else(|e| die(format!("write {}: {e}", write_dir.display())))
    } else {
        let body = BLANK_TEMPLATE.replace("{FLAVOR}", flavor);
        let target = write_dir.join(CONFIG_FILENAME);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .unwrap_or_else(|e| die(format!("create {}: {e}", parent.display())));
        }
        std::fs::write(&target, body)
            .unwrap_or_else(|e| die(format!("write {}: {e}", target.display())));
        target
    };
    log(format!("marked {} as flavor={flavor}", path.display()));
    if !docker::image_exists(&image_name(flavor)) {
        build_image(flavor, false);
    }
    log("ready. run 'sbx' to enter.");
    log(format!(
        "extra deps: create {}/Dockerfile starting with 'FROM {}'",
        write_dir.display(),
        image_name(flavor)
    ));
}
