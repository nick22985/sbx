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
            let root_s = root.display().to_string();
            if let Some(c) = docker::find_running_container(&flavor, &pname) {
                log(format!("trivy fs {root_s} (via {c})"));
                let err = Command::new("docker")
                    .args(["exec", "-i", &c, "trivy", "fs", "--no-progress", &root_s])
                    .exec();
                die(format!("exec: {err}"));
            }
            let image = resolve_image(&flavor, &root, false);
            log(format!("trivy fs {root_s} (transient {image})"));
            let uid = nix_uid();
            let gid = nix_gid();
            let err = Command::new("docker")
                .args(["run", "--rm", "-i"])
                .args(["--user", &format!("{uid}:{gid}")])
                .args(["--cap-drop=ALL", "--security-opt=no-new-privileges"])
                .arg("-v")
                .arg(format!("{root_s}:{root_s}:ro"))
                .args(["-w", &root_s, &image, "trivy", "fs", "--no-progress", &root_s])
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
