use std::path::Path;
use std::process::Command;

use crate::project::{project_flavor, project_name};
use crate::service;
use crate::util::{die, log};
use crate::vpn::{self, sidecar_name};

pub fn run(cwd: &Path) {
    let (flavor, root) =
        project_flavor(cwd).unwrap_or_else(|| die("no .sbx/flavor here."));
    let pname = project_name(&root);
    let out = Command::new("docker")
        .args([
            "ps",
            "--filter",
            &format!("name=^sbx-{flavor}-{pname}-"),
            "--format",
            "{{.Names}}",
        ])
        .output();
    let names: Vec<String> = out
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() {
        log(format!("no running container for {flavor}/{pname}"));
    }
    for c in &names {
        log(format!("stopping {c}"));
        let _ = Command::new("docker").args(["stop", c]).status();
    }
    service::stop_all_for_project(&pname);
    vpn::stop_sidecar(&sidecar_name(&pname));
}
