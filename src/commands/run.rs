use std::path::Path;

use crate::commands::session;
use crate::config::Config;
use crate::docker;
use crate::project::{project_flavor, project_name};
use crate::util::{die, log};

pub fn run(cwd: &Path) -> i32 {
    let (flavor, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));
    let pname = project_name(&root);
    if let Some(c) = docker::find_running_container(&flavor, &pname) {
        die(format!(
            "container already running for {flavor}/{pname}: {c}\n  - stop it:  sbx stop\n  - attach:   sbx shell"
        ));
    }
    let cmd = Config::load_or_default(&root)
        .start
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if cmd.is_empty() {
        die("no start command. set one with: sbx config start \"<cmd>\"");
    }
    let entry = vec!["/bin/bash".into(), "-lc".into(), cmd];
    log(format!("running: {:?}", entry));
    session::run_session(&flavor, &root, entry)
}
