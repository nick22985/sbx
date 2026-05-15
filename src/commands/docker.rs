use std::fs;
use std::path::Path;

use crate::docker::{host_docker_socket, project_docker_enabled};
use crate::project::{project_flavor, sbx_write_dir};
use crate::util::{die, log};

pub enum Action {
    On,
    Off,
    Status,
}

pub fn run(cwd: &Path, action: Action) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
    let write_dir = sbx_write_dir(&root);
    let write_file = write_dir.join("docker");
    match action {
        Action::On => {
            if let Err(e) = fs::create_dir_all(&write_dir) {
                die(format!("mkdir {}: {e}", write_dir.display()));
            }
            if !write_file.is_file() {
                let _ = fs::write(&write_file, "");
            }
            log(format!(
                "docker socket forwarding enabled for this project ({})",
                write_file.display()
            ));
            log("WARNING: mounting the host docker socket grants root-equivalent access to the host. anything in the container can break out via `docker run --privileged`.");
        }
        Action::Off => {
            let _ = fs::remove_file(&write_file);
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
