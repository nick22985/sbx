use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::docker::{self, Network, PortSpec, RunSpec};
use crate::flavor::{build_image, image_exists_or_build, image_name};
use crate::project::{project_name, sbx_file};
use crate::util::{config_dir, die, expand_tilde, home_dir, log};

const FLAVOR: &str = "claude";

pub fn run(
    cwd: &Path,
    args: Vec<String>,
    shell: bool,
    cli_mounts: Vec<String>,
    cli_profile: Option<String>,
    safe: bool,
    no_rc: bool,
) -> i32 {
    image_exists_or_build(FLAVOR);

    let entry: Vec<String> = if shell {
        vec!["/bin/bash".into()]
    } else {
        let mut v = vec!["claude".into()];
        let already = args.iter().any(|a| a == "--dangerously-skip-permissions");
        if !safe && !already {
            v.push("--dangerously-skip-permissions".into());
        }
        let rc_off = no_rc
            || std::env::var("SBX_REMOTE_CONTROL")
                .map(|v| v == "0")
                .unwrap_or(false);
        let rc_already = args.iter().any(|a| a == "--remote-control" || a == "--rc");
        if !rc_off && !rc_already {
            let name = format!("{}-{}", project_name(cwd), std::process::id());
            v.push("--remote-control".into());
            v.push(name);
        }
        v.extend(args);
        v
    };

    let extra_mounts = resolve_extra_mounts(cwd, &cli_mounts);
    let profile = resolve_profile(cwd, cli_profile.as_deref());
    let extra_host_args = build_claude_mount_args(profile.as_deref());

    let image = image_name(FLAVOR);
    let spec = RunSpec {
        image: &image,
        flavor: FLAVOR,
        project_root: cwd,
        entry,
        network: Network::Bridge,
        use_hostname: true,
        publish_ports: PortSpec::default(),
        extra_host_args,
        host_workspace: true,
        extra_mounts,
        container_home: home_dir(),
    };
    docker::run_container(spec)
}

pub fn build(no_cache: bool) {
    log(format!("building sbx-{FLAVOR}:latest"));
    build_image(FLAVOR, no_cache);
}

fn resolve_extra_mounts(cwd: &Path, cli: &[String]) -> Vec<PathBuf> {
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut out = Vec::new();
    let cwd_canon = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let push = |raw: &str, out: &mut Vec<PathBuf>, seen: &mut BTreeSet<PathBuf>| {
        let trimmed = raw.split('#').next().unwrap_or("").trim();
        if trimmed.is_empty() {
            return;
        }
        let path = expand_tilde(trimmed);
        let canon = std::fs::canonicalize(&path).unwrap_or(path);
        if canon == cwd_canon {
            return;
        }
        if seen.insert(canon.clone()) {
            out.push(canon);
        }
    };
    let global = config_dir().join("claude-mounts");
    if let Ok(contents) = std::fs::read_to_string(&global) {
        for line in contents.lines() {
            push(line, &mut out, &mut seen);
        }
    }
    let file = sbx_file(cwd, "claude-mounts");
    if let Ok(contents) = std::fs::read_to_string(&file) {
        for line in contents.lines() {
            push(line, &mut out, &mut seen);
        }
    }
    for raw in cli {
        push(raw, &mut out, &mut seen);
    }
    out
}

pub fn profiles_root() -> PathBuf {
    config_dir().join("claude-profiles")
}

pub fn profile_dir(name: &str) -> PathBuf {
    profiles_root().join(name)
}

pub fn profile_exists(name: &str) -> bool {
    profile_dir(name).is_dir()
}

pub fn list_profiles() -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(profiles_root()) else {
        return out;
    };
    for e in entries.flatten() {
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false)
            && let Some(name) = e.file_name().to_str()
        {
            out.push(name.to_string());
        }
    }
    out.sort();
    out
}

fn validate_profile_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("profile name cannot be empty".into());
    }
    if name.contains('/') || name.contains("..") || name.starts_with('.') {
        return Err(format!("invalid profile name: {name}"));
    }
    Ok(())
}

pub fn add_profile(name: &str) {
    if let Err(e) = validate_profile_name(name) {
        die(e);
    }
    let dir = profile_dir(name);
    if dir.exists() {
        die(format!("profile already exists: {name}"));
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        die(format!("create {}: {e}", dir.display()));
    }
    let host_json = home_dir().join(".claude.json");
    let target_json = dir.join(".claude.json");
    if host_json.is_file() {
        if let Err(e) = std::fs::copy(&host_json, &target_json) {
            die(format!("seed .claude.json: {e}"));
        }
    } else {
        if let Err(e) = std::fs::write(&target_json, "{}\n") {
            die(format!("write {}: {e}", target_json.display()));
        }
    }
    log(format!("created profile: {name} at {}", dir.display()));
    log(format!(
        "next: sbx claude -p {name}   (first run drops to /login)"
    ));
}

pub fn remove_profile(name: &str) {
    if let Err(e) = validate_profile_name(name) {
        die(e);
    }
    let dir = profile_dir(name);
    if !dir.is_dir() {
        die(format!("no such profile: {name}"));
    }
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        die(format!("remove {}: {e}", dir.display()));
    }
    log(format!("removed profile: {name}"));
}

pub fn resolve_profile(cwd: &Path, cli: Option<&str>) -> Option<String> {
    if let Some(raw) = cli {
        let name = raw.trim();
        if name.is_empty() {
            return None;
        }
        if !profile_exists(name) {
            die(format!(
                "no such profile: {name}  (have: {})",
                list_profiles().join(", ")
            ));
        }
        return Some(name.to_string());
    }
    let pin = sbx_file(cwd, "claude-profile");
    if let Ok(contents) = std::fs::read_to_string(&pin) {
        let name = contents.trim();
        if !name.is_empty() {
            if !profile_exists(name) {
                die(format!(
                    "{} pins profile {name}, but it doesn't exist (have: {})",
                    pin.display(),
                    list_profiles().join(", ")
                ));
            }
            return Some(name.to_string());
        }
    }
    None
}

pub fn print_profile_list(cwd: &Path) {
    let current = resolve_profile(cwd, None);
    let profiles = list_profiles();
    if profiles.is_empty() {
        log("no profiles yet — create one with: sbx claude profile add NAME");
        return;
    }
    for p in profiles {
        let marker = if Some(&p) == current.as_ref() {
            "*"
        } else {
            " "
        };
        println!("{marker} {p}");
    }
}

pub fn print_current_profile(cwd: &Path, cli_profile: Option<&str>) {
    match resolve_profile(cwd, cli_profile) {
        Some(name) => println!("{name}"),
        None => println!("(none — using host ~/.claude.json)"),
    }
}

fn build_claude_mount_args(profile: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    let h = home_dir();
    let hs = h.display().to_string();

    out.push("-v".into());
    out.push(format!("{hs}/.claude:{hs}/.claude"));

    let claude_json_src: PathBuf = if let Some(name) = profile {
        profile_dir(name).join(".claude.json")
    } else {
        h.join(".claude.json")
    };
    if claude_json_src.is_file() {
        out.push("-v".into());
        out.push(format!("{}:{}/.claude.json", claude_json_src.display(), hs));
    }

    if let Some(name) = profile {
        let creds = profile_dir(name).join(".credentials.json");
        if !creds.exists() {
            if let Some(parent) = creds.parent()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                die(format!("create {}: {e}", parent.display()));
            }
            if let Err(e) = std::fs::write(&creds, "") {
                die(format!("create {}: {e}", creds.display()));
            }
        }
        out.push("-v".into());
        out.push(format!(
            "{}:{}/.claude/.credentials.json",
            creds.display(),
            hs
        ));
        log(format!("claude profile: {name}"));
    }

    out
}
