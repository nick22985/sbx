use std::path::Path;

use crate::commands::session;
use crate::docker;
use crate::flavor::{is_flavor, is_internal_flavor};
use crate::project::{project_flavor, project_name};
use crate::util::die;

pub fn from_project(cwd: &Path, flavor: Option<String>, entry: Vec<String>) -> i32 {
    if let Some(f) = flavor {
        if is_internal_flavor(&f) {
            die(format!(
                "'{f}' isn't a project flavor — use `sbx {f}` to launch it directly"
            ));
        }
        if !is_flavor(&f) {
            die(format!("unknown flavor: {f}"));
        }
        return attach_or_run(&f, cwd, entry);
    }
    let (flavor, root) = project_flavor(cwd).unwrap_or_else(|| {
        die("no .sbx/flavor here. run 'sbx init <flavor>' first, or 'sbx <flavor>' for ad-hoc.")
    });
    attach_or_run(&flavor, &root, entry)
}

pub fn ad_hoc(cwd: &Path, flavor: &str) -> i32 {
    if is_internal_flavor(flavor) {
        die(format!(
            "'{flavor}' isn't a project flavor — use `sbx {flavor}` to launch it directly"
        ));
    }
    if !is_flavor(flavor) {
        die(format!("unknown command or flavor: {flavor}"));
    }
    attach_or_run(flavor, cwd, Vec::new())
}

fn attach_or_run(flavor: &str, project_root: &Path, entry: Vec<String>) -> i32 {
    let entry = wrap_login(entry);
    let pname = project_name(project_root);
    if let Some(c) = docker::find_running_container(flavor, &pname) {
        let err = docker::exec_into(&c, project_root, &entry);
        die(format!("exec: {err}"));
    }
    session::run_session(flavor, project_root, entry)
}

fn wrap_login(entry: Vec<String>) -> Vec<String> {
    if entry.is_empty() {
        return entry;
    }
    let mut out = vec![
        "/bin/bash".to_string(),
        "-lc".to_string(),
        "exec \"$@\"".to_string(),
        "sbx-shell".to_string(),
    ];
    out.extend(entry);
    out
}
