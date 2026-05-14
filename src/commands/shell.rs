use std::path::Path;

use crate::commands::session;
use crate::docker;
use crate::flavor::{is_flavor, is_internal_flavor};
use crate::project::{project_flavor, project_name};
use crate::util::die;

pub fn from_project(cwd: &Path) -> i32 {
    let (flavor, root) = project_flavor(cwd).unwrap_or_else(|| {
        die("no .sbx/flavor here. run 'sbx init <flavor>' first, or 'sbx <flavor>' for ad-hoc.")
    });
    attach_or_run(&flavor, &root)
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
    attach_or_run(flavor, cwd)
}

fn attach_or_run(flavor: &str, project_root: &Path) -> i32 {
    let pname = project_name(project_root);
    if let Some(c) = docker::find_running_container(flavor, &pname) {
        let err = docker::exec_into(&c, &[]);
        die(format!("exec: {err}"));
    }
    session::run_session(flavor, project_root, Vec::new())
}
