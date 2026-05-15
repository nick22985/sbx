use std::fs;
use std::path::Path;

use crate::project::{project_flavor, sbx_file, sbx_write_dir};
use crate::service::{BUILTIN_SERVICES, project_services, service_template};
use crate::util::{die, log};

pub enum Action<'a> {
    List,
    Add(&'a str),
    Remove(&'a str),
}

pub fn run(cwd: &Path, action: Action<'_>) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
    let read_file = sbx_file(&root, "services");
    let write_dir = sbx_write_dir(&root);
    let write_file = write_dir.join("services");

    match action {
        Action::List => {
            let svcs = project_services(&root);
            if svcs.is_empty() {
                log("no services configured");
                return;
            }
            log(format!("from {}:", read_file.display()));
            for s in svcs {
                println!("{s}");
            }
        }
        Action::Add(name) => {
            if service_template(name).is_none() {
                die(format!(
                    "unknown service: '{name}' (built-ins: {}; or pass an image like 'ghcr.io/foo/bar:tag')",
                    BUILTIN_SERVICES.join(" ")
                ));
            }
            if let Err(e) = fs::create_dir_all(&write_dir) {
                die(format!("mkdir {}: {e}", write_dir.display()));
            }
            let mut current = fs::read_to_string(&write_file).unwrap_or_default();
            for line in current.lines() {
                let cleaned = line.split('#').next().unwrap_or("").trim();
                if cleaned == name {
                    log(format!("already present: {name}"));
                    return;
                }
            }
            if !current.ends_with('\n') && !current.is_empty() {
                current.push('\n');
            }
            current.push_str(name);
            current.push('\n');
            if let Err(e) = fs::write(&write_file, current) {
                die(format!("write {}: {e}", write_file.display()));
            }
            log(format!("added {name} to {}", write_file.display()));
        }
        Action::Remove(name) => {
            if !write_file.is_file() {
                die(format!("no {}", write_file.display()));
            }
            let content = fs::read_to_string(&write_file).unwrap_or_default();
            let kept: Vec<&str> = content
                .lines()
                .filter(|line| {
                    let cleaned = line.split('#').next().unwrap_or("").trim();
                    cleaned != name
                })
                .collect();
            let mut out = kept.join("\n");
            if !out.is_empty() {
                out.push('\n');
            }
            if let Err(e) = fs::write(&write_file, out) {
                die(format!("write {}: {e}", write_file.display()));
            }
            log(format!("removed {name} from {}", write_file.display()));
        }
    }
}
