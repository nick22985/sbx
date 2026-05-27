use std::path::Path;

use crate::config::{Config, GlobalConfig};
use crate::project::project_flavor;
use crate::util::{die, expand_tilde, log};

pub enum Action<'a> {
    List,
    Add(&'a str),
    Remove(&'a str),
}

const USAGE: &str = "usage: sbx config mount add <host>[:<container>][:ro]";

pub fn run(cwd: &Path, action: Action<'_>, global: bool) {
    if global {
        run_global(action);
        return;
    }

    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));

    match action {
        Action::List => list(
            &Config::load_or_default(&root).mounts,
            "no mounts configured",
        ),
        Action::Add(spec) => {
            let spec = clean_spec(spec);
            if Config::load_or_default(&root)
                .mounts
                .iter()
                .any(|m| m == spec)
            {
                log(format!("mount already present: {spec}"));
                return;
            }
            warn_if_missing(spec);
            let path = Config::edit(&root, |c| c.mounts.push(spec.to_string()))
                .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!("added {spec} to {}", path.display()));
        }
        Action::Remove(spec) => {
            let spec = clean_spec(spec);
            let path = Config::edit(&root, |c| c.mounts.retain(|m| !mount_matches(m, spec)))
                .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!("removed {spec} from {}", path.display()));
        }
    }
}

fn run_global(action: Action<'_>) {
    match action {
        Action::List => list(
            &GlobalConfig::load_or_default().mounts,
            "no global mounts configured",
        ),
        Action::Add(spec) => {
            let spec = clean_spec(spec);
            let mut cfg = GlobalConfig::load_or_default();
            if cfg.mounts.iter().any(|m| m == spec) {
                log(format!("global mount already present: {spec}"));
                return;
            }
            warn_if_missing(spec);
            cfg.mounts.push(spec.to_string());
            let path = cfg
                .save()
                .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!("added {spec} to {}", path.display()));
        }
        Action::Remove(spec) => {
            let spec = clean_spec(spec);
            let mut cfg = GlobalConfig::load_or_default();
            cfg.mounts.retain(|m| !mount_matches(m, spec));
            let path = cfg
                .save()
                .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!("removed {spec} from {}", path.display()));
        }
    }
}

fn list(mounts: &[String], empty_msg: &str) {
    if mounts.is_empty() {
        log(empty_msg);
        return;
    }
    for m in mounts {
        println!("{m}");
    }
}

fn clean_spec(spec: &str) -> &str {
    let spec = spec.trim();
    if spec.is_empty() {
        die(USAGE);
    }
    spec
}

fn mount_matches(stored: &str, spec: &str) -> bool {
    stored == spec || stored.split(':').next().map(str::trim) == Some(spec)
}

fn warn_if_missing(spec: &str) {
    let host = spec.split(':').next().unwrap_or("").trim();
    if host.is_empty() {
        return;
    }
    let resolved = expand_tilde(host);
    if !resolved.exists() {
        log(format!(
            "note: host path does not exist yet, will be skipped until created: {}",
            resolved.display()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_matches_exact_line() {
        assert!(mount_matches("~/foo:/bar:ro", "~/foo:/bar:ro"));
    }

    #[test]
    fn mount_matches_by_host_segment() {
        assert!(mount_matches("~/foo:/bar:ro", "~/foo"));
        assert!(mount_matches("/etc/foo", "/etc/foo"));
    }

    #[test]
    fn mount_matches_rejects_partial_or_container_side() {
        assert!(!mount_matches("~/foo:/bar", "/bar"));
        assert!(!mount_matches("~/foobar", "~/foo"));
    }
}
