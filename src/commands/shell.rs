use std::path::Path;

use crate::commands::session;
use crate::config::FlavorConfig;
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
        die("no .sbx/config.toml here. run 'sbx init <flavor>' first, or 'sbx <flavor>' for ad-hoc.")
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
    let entry = flavor_start_entry(flavor);
    attach_or_run(flavor, cwd, entry)
}

fn flavor_start_entry(flavor: &str) -> Vec<String> {
    match FlavorConfig::load_or_default(flavor).start {
        Some(s) if !s.trim().is_empty() => {
            vec!["/bin/bash".to_string(), "-lc".to_string(), s]
        }
        _ => Vec::new(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::set_test_paths;
    use std::path::PathBuf;

    fn tmp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("sbx-shell-{label}-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn flavor_start_entry_empty_when_unset() {
        let cfg = tmp_dir("start-unset-cfg");
        let home = tmp_dir("start-unset-home");
        let _g = set_test_paths(cfg, home);
        assert!(flavor_start_entry("nvim").is_empty());
    }

    #[test]
    fn flavor_start_entry_builds_bash_lc_when_set() {
        let cfg = tmp_dir("start-set-cfg");
        let home = tmp_dir("start-set-home");
        let _g = set_test_paths(cfg, home);
        FlavorConfig {
            start: Some("nvim .".into()),
            ..Default::default()
        }
        .save("nvim")
        .unwrap();
        assert_eq!(
            flavor_start_entry("nvim"),
            vec![
                "/bin/bash".to_string(),
                "-lc".to_string(),
                "nvim .".to_string()
            ]
        );
    }

    #[test]
    fn flavor_start_entry_treats_blank_as_unset() {
        let cfg = tmp_dir("start-blank-cfg");
        let home = tmp_dir("start-blank-home");
        let _g = set_test_paths(cfg, home);
        FlavorConfig {
            start: Some("   ".into()),
            ..Default::default()
        }
        .save("npm")
        .unwrap();
        assert!(flavor_start_entry("npm").is_empty());
    }

    #[test]
    fn wrap_login_preserves_empty_entry() {
        assert!(wrap_login(Vec::new()).is_empty());
    }

    #[test]
    fn wrap_login_wraps_non_empty_entry() {
        let wrapped = wrap_login(vec!["nvim".into(), ".".into()]);
        assert_eq!(
            wrapped,
            vec![
                "/bin/bash".to_string(),
                "-lc".to_string(),
                "exec \"$@\"".to_string(),
                "sbx-shell".to_string(),
                "nvim".to_string(),
                ".".to_string(),
            ]
        );
    }
}
