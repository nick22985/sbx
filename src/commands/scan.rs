use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use crate::docker;
use crate::flavor::{nix_gid, nix_uid, resolve_image};
use crate::project::{project_flavor, project_name};
use crate::util::{die, log};

pub enum Target {
    Fs,
    Image,
}

pub fn run(cwd: &Path, target: Target) {
    let (flavor, root) =
        project_flavor(cwd).unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
    let pname = project_name(&root);
    match target {
        Target::Fs => {
            if let Some(c) = docker::find_running_container(&flavor, &pname) {
                log(format!("trivy fs /workspace (via {c})"));
                let err = Command::new("docker")
                    .args(["exec", "-i", &c, "trivy", "fs", "--no-progress", "/workspace"])
                    .exec();
                die(format!("exec: {err}"));
            }
            let image = resolve_image(&flavor, &root, false);
            log(format!("trivy fs /workspace (transient {image})"));
            let uid = nix_uid();
            let gid = nix_gid();
            let err = Command::new("docker")
                .args(["run", "--rm", "-i"])
                .args(["--user", &format!("{uid}:{gid}")])
                .args(["--cap-drop=ALL", "--security-opt=no-new-privileges"])
                .arg("-v")
                .arg(format!("{}:/workspace:ro", root.display()))
                .args(["-w", "/workspace", &image, "trivy", "fs", "--no-progress", "/workspace"])
                .exec();
            die(format!("exec: {err}"));
        }
        Target::Image => {
            let image = resolve_image(&flavor, &root, false);
            log(format!("trivy image {image}"));
            let err = Command::new("trivy")
                .args(["image", "--no-progress", &image])
                .exec();
            die(format!("exec: {err}"));
        }
    }
}
