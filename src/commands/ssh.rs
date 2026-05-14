use std::fs;
use std::path::Path;

use crate::docker::project_ssh_enabled;
use crate::project::{project_flavor, sbx_write_dir};
use crate::util::{die, log};

pub enum Action {
    On,
    Off,
    Status,
}

pub fn run(cwd: &Path, action: Action) {
    let (_, root) =
        project_flavor(cwd).unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
    let write_dir = sbx_write_dir(&root);
    let write_file = write_dir.join("ssh");
    match action {
        Action::On => {
            if let Err(e) = fs::create_dir_all(&write_dir) {
                die(format!("mkdir {}: {e}", write_dir.display()));
            }
            if !write_file.is_file() {
                let _ = fs::write(&write_file, "");
            }
            log(format!(
                "ssh agent forwarding enabled for this project ({})",
                write_file.display()
            ));
            log("next container start will mount $SSH_AUTH_SOCK + ~/.ssh/{config,known_hosts} (ro)");
        }
        Action::Off => {
            let _ = fs::remove_file(&write_file);
            log("ssh agent forwarding disabled");
        }
        Action::Status => {
            if project_ssh_enabled(&root) {
                log("ssh agent forwarding: ON");
                match std::env::var("SSH_AUTH_SOCK") {
                    Ok(s) if !s.is_empty() => log(format!("host SSH_AUTH_SOCK={s}")),
                    _ => log("warning: SSH_AUTH_SOCK not set on host"),
                }
            } else {
                log("ssh agent forwarding: off");
            }
        }
    }
}
