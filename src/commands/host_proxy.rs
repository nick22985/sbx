use std::fs;
use std::path::Path;

use crate::host_proxy;
use crate::project::{project_flavor, project_name, sbx_write_dir};
use crate::util::{die, log};

pub enum Action<'a> {
    On,
    Off,
    Status,
    Logs { follow: bool },
    Stop,
    Allow(&'a str),
    Disallow(&'a str),
    List,
    Reload,
}

pub fn run(cwd: &Path, action: Action<'_>) {
    match action {
        Action::On => {
            let root = require_project(cwd);
            let write_dir = sbx_write_dir(&root);
            let write_file = write_dir.join("host-proxy");
            if let Err(e) = fs::create_dir_all(&write_dir) {
                die(format!("mkdir {}: {e}", write_dir.display()));
            }
            if !write_file.is_file() {
                let _ = fs::write(&write_file, "");
            }
            log(format!(
                "host-proxy enabled for this project ({})",
                write_file.display()
            ));
            log(format!(
                "  sandbox env: https_proxy=http://host.docker.internal:{}",
                host_proxy::PORT
            ));
            let hosts = host_proxy::read_allowed_hosts(&root);
            if hosts.is_empty() {
                log(
                    "  allowlist empty → unrestricted for this project. add hosts with: sbx host-proxy allow <host>",
                );
            } else {
                log(format!("  allowlist: {}", hosts.join(", ")));
            }
        }
        Action::Off => {
            let root = require_project(cwd);
            let write_dir = sbx_write_dir(&root);
            let write_file = write_dir.join("host-proxy");
            let _ = fs::remove_file(&write_file);
            host_proxy::remove_project_fragment(&project_name(&root));
            if let Err(e) = host_proxy::apply_config() {
                log(format!("host-proxy: {e}"));
            }
            log("host-proxy disabled for this project");
        }
        Action::Status => {
            let project_state = project_flavor(cwd).map(|(_, root)| {
                (
                    host_proxy::is_enabled(&root),
                    host_proxy::read_allowed_hosts(&root),
                )
            });
            match project_state {
                Some((true, hosts)) => {
                    log("project host-proxy marker: ON");
                    if hosts.is_empty() {
                        log("  project allowlist: (empty → unrestricted)");
                    } else {
                        log(format!("  project allowlist: {}", hosts.join(", ")));
                    }
                }
                Some((false, _)) => log("project host-proxy marker: off"),
                None => log("project host-proxy marker: (not in an sbx project)"),
            }
            let state = crate::docker::state_line(host_proxy::SIDECAR);
            if host_proxy::sidecar_running() {
                log(format!(
                    "host-proxy sidecar: {state} — http://host.docker.internal:{}",
                    host_proxy::PORT
                ));
            } else {
                log(format!("host-proxy sidecar: {state}"));
            }
            let merged = host_proxy::merged_allowlist();
            if merged.is_empty() {
                log("  merged allowlist: (empty → sidecar unrestricted)");
            } else {
                log(format!(
                    "  merged allowlist ({}): {}",
                    merged.len(),
                    merged.join(", ")
                ));
            }
            log(format!("  config: {}", host_proxy::config_path().display()));
            log(format!("  filter: {}", host_proxy::filter_path().display()));
        }
        Action::Logs { follow } => {
            if !host_proxy::sidecar_exists() {
                die("host-proxy sidecar not running");
            }
            crate::docker::tail_logs(host_proxy::SIDECAR, follow);
        }
        Action::Stop => {
            if host_proxy::force_stop() {
                log("host-proxy sidecar stopped");
            } else {
                log("host-proxy sidecar not running");
            }
        }
        Action::Allow(host) => {
            let root = require_project(cwd);
            ensure_marker(&root);
            match host_proxy::add_allowed_host(&root, host) {
                Ok(true) => log(format!("added '{host}' to project allowlist")),
                Ok(false) => log(format!("'{host}' already in project allowlist")),
                Err(e) => die(format!("host-proxy allow: {e}")),
            }
            write_fragment_and_apply(&root);
        }
        Action::Disallow(host) => {
            let root = require_project(cwd);
            match host_proxy::remove_allowed_host(&root, host) {
                Ok(true) => log(format!("removed '{host}' from project allowlist")),
                Ok(false) => log(format!("'{host}' not in project allowlist")),
                Err(e) => die(format!("host-proxy disallow: {e}")),
            }
            write_fragment_and_apply(&root);
        }
        Action::List => {
            let root = require_project(cwd);
            let hosts = host_proxy::read_allowed_hosts(&root);
            if hosts.is_empty() {
                log("project allowlist: (empty → unrestricted)");
            } else {
                for h in hosts {
                    println!("{h}");
                }
            }
        }
        Action::Reload => match host_proxy::apply_config() {
            Ok(()) => {
                if !host_proxy::sidecar_running() {
                    log("host-proxy sidecar not running (config rewritten on disk)");
                }
            }
            Err(e) => die(format!("host-proxy reload: {e}")),
        },
    }
}

fn require_project(cwd: &Path) -> std::path::PathBuf {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
    root
}

fn ensure_marker(root: &Path) {
    let write_dir = sbx_write_dir(root);
    let write_file = write_dir.join("host-proxy");
    if let Err(e) = fs::create_dir_all(&write_dir) {
        die(format!("mkdir {}: {e}", write_dir.display()));
    }
    if !write_file.is_file() {
        let _ = fs::write(&write_file, "");
    }
}

fn write_fragment_and_apply(root: &Path) {
    let pname = project_name(root);
    let hosts = host_proxy::read_allowed_hosts(root);
    if let Err(e) = host_proxy::write_project_fragment(&pname, &hosts) {
        log(format!("host-proxy: {e}"));
        return;
    }
    if let Err(e) = host_proxy::apply_config() {
        log(format!("host-proxy: {e}"));
    }
}
