use std::path::Path;

use crate::config::Config;
use crate::docker::project_ssh_enabled;
use crate::project::project_flavor;
use crate::util::{die, log};

pub enum Action {
    On,
    Off,
    Status,
}

pub fn run(cwd: &Path, action: Action) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));
    match action {
        Action::On => {
            let path = Config::edit(&root, |c| c.ssh = true)
                .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!(
                "ssh agent forwarding enabled for this project ({})",
                path.display()
            ));
            log(
                "next container start will mount $SSH_AUTH_SOCK + ~/.ssh/{config,known_hosts} (ro)",
            );
        }
        Action::Off => {
            let _ = Config::edit(&root, |c| c.ssh = false);
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
