use std::path::Path;

use crate::config::Config;
use crate::docker::{host_docker_socket, project_docker_enabled};
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
            let path = Config::edit(&root, |c| c.docker = true)
                .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!(
                "docker socket forwarding enabled for this project ({})",
                path.display()
            ));
            log(
                "WARNING: mounting the host docker socket grants root-equivalent access to the host. anything in the container can break out via `docker run --privileged`.",
            );
        }
        Action::Off => {
            let _ = Config::edit(&root, |c| c.docker = false);
            log("docker socket forwarding disabled");
        }
        Action::Status => {
            if project_docker_enabled(&root) {
                log("docker socket forwarding: ON");
                let sock = host_docker_socket();
                log(format!("host socket: {}", sock.display()));
            } else {
                log("docker socket forwarding: off");
            }
        }
    }
}
