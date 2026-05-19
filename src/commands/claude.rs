use std::path::{Path, PathBuf};

use crate::docker::{self, Network, PortSpec, RunSpec};
use crate::flavor::{build_image, image_exists_or_build, image_name};
use crate::mounts;
use crate::project::{project_base_name, project_name, worktree_suffix};
use crate::util::{config_dir, die, home_dir, log};

const FLAVOR: &str = "claude";

pub fn run(
    cwd: &Path,
    args: Vec<String>,
    shell: bool,
    cli_mounts: Vec<String>,
    cli_profile: Option<String>,
    safe: bool,
    rc: bool,
    cli_docker: bool,
) -> i32 {
    let mut cli_mounts = cli_mounts;
    let mut cli_profile = cli_profile;
    let mut safe = safe;
    let mut rc = rc;
    let mut cli_docker = cli_docker;
    let args = extract_sbx_flags(
        args,
        &mut cli_mounts,
        &mut cli_profile,
        &mut safe,
        &mut rc,
        &mut cli_docker,
    );

    docker::ensure_ssh_agent_ready(cwd);
    image_exists_or_build(FLAVOR);

    let entry: Vec<String> = if shell {
        vec!["/bin/bash".into()]
    } else {
        let mut v = vec!["claude".into()];
        let already = args.iter().any(|a| a == "--dangerously-skip-permissions");
        if !safe && !already {
            v.push("--dangerously-skip-permissions".into());
        }
        let rc_on = rc
            || std::env::var("SBX_REMOTE_CONTROL")
                .map(|v| v == "1")
                .unwrap_or(false);
        let rc_already = args.iter().any(|a| a == "--remote-control" || a == "--rc");
        if rc_on && !rc_already {
            let name = format!("{}-{}", project_name(cwd), std::process::id());
            v.push("--remote-control".into());
            v.push(name);
        }
        v.extend(args);
        v
    };

    let extra_mounts = mounts::resolve(cwd, &home_dir(), &cli_mounts, Some("claude"));
    let profile = resolve_profile(cwd, cli_profile.as_deref());
    let extra_host_args = build_claude_mount_args(profile.as_deref());

    let mount_docker_socket = cli_docker
        || std::env::var("SBX_DOCKER")
            .map(|v| v == "1")
            .unwrap_or(false);

    let image = image_name(FLAVOR);
    let extra_env = vec![
        ("SBX_PROJECT".into(), project_name(cwd)),
        ("SBX_PROJECT_BASE".into(), project_base_name(cwd)),
        (
            "SBX_WORKTREE".into(),
            worktree_suffix(cwd).unwrap_or_default(),
        ),
    ];
    let spec = RunSpec {
        image: &image,
        flavor: FLAVOR,
        project_root: cwd,
        entry,
        network: Network::Bridge,
        use_hostname: true,
        publish_ports: PortSpec::default(),
        extra_host_args,
        extra_mounts,
        container_home: home_dir(),
        labels: Vec::new(),
        mount_docker_socket,
        extra_env,
    };
    docker::run_container(spec)
}

pub fn build(no_cache: bool) {
    log(format!("building sbx-{FLAVOR}:latest"));
    build_image(FLAVOR, no_cache);
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
    if let Some(name) = crate::config::Config::load_or_default(cwd).claude.profile {
        let name = name.trim();
        if !name.is_empty() {
            if !profile_exists(name) {
                die(format!(
                    ".sbx/config.toml pins claude profile {name}, but it doesn't exist (have: {})",
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
        log("no profiles yet - create one with: sbx claude profile add NAME");
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
        None => println!("(none, using host ~/.claude.json)"),
    }
}

fn extract_sbx_flags(
    args: Vec<String>,
    mounts: &mut Vec<String>,
    profile: &mut Option<String>,
    safe: &mut bool,
    rc: &mut bool,
    docker: &mut bool,
) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut iter = args.into_iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--docker" => *docker = true,
            "--safe" | "-s" => *safe = true,
            "--rc" => *rc = true,
            "--profile" | "-p" => match iter.next() {
                Some(v) => *profile = Some(v),
                None => die("--profile requires a value"),
            },
            "--mount" | "-m" => match iter.next() {
                Some(v) => mounts.push(v),
                None => die("--mount requires a value"),
            },
            _ => {
                if let Some(v) = a.strip_prefix("--profile=") {
                    *profile = Some(v.to_string());
                } else if let Some(v) = a.strip_prefix("--mount=") {
                    mounts.push(v.to_string());
                } else {
                    out.push(a);
                }
            }
        }
    }
    out
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
