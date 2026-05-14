use std::path::Path;

use crate::commands::session;
use crate::docker;
use crate::project::{project_flavor, project_name, sbx_file};
use crate::util::{die, log};

pub fn run(cwd: &Path) -> i32 {
    let (flavor, root) =
        project_flavor(cwd).unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
    let start = sbx_file(&root, "start");
    if !start.is_file() {
        die("no .sbx/start file. put a shell command there (e.g. 'npm run dev').");
    }
    let pname = project_name(&root);
    if let Some(c) = docker::find_running_container(&flavor, &pname) {
        die(format!(
            "container already running for {flavor}/{pname}: {c}\n  - stop it:  sbx stop\n  - attach:   sbx shell"
        ));
    }
    let cmd = std::fs::read_to_string(&start).unwrap_or_default();
    let cmd = cmd.trim().to_string();
    if cmd.is_empty() {
        die("empty .sbx/start");
    }
    let entry = vec!["/bin/bash".into(), "-lc".into(), cmd];
    log(format!("running: {:?}", entry));
    session::run_session(&flavor, &root, entry)
}
