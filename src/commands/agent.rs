use std::path::Path;

use crate::commands::profiles;
use crate::config::{Agent, FlavorConfig};
use crate::docker::{self, Network, PortSpec, RunSpec};
use crate::flavor::{build_image, container_home, image_exists_or_build, image_name};
use crate::mounts;
use crate::project::{project_base_name, project_name, worktree_suffix};
use crate::util::{die, log};

#[derive(Default)]
pub struct Invocation {
    pub args: Vec<String>,
    pub shell: bool,
    pub mounts: Vec<String>,
    pub safe: bool,
    pub docker: bool,
    pub profile: Option<String>,
    pub rc: bool,
    pub no_rc: bool,
    pub gpg: bool,
}

pub fn is_agent(flavor: &str) -> bool {
    FlavorConfig::load_or_default(flavor).agent.is_some()
}

pub fn build(flavor: &str, no_cache: bool) {
    log(format!("building {}", image_name(flavor)));
    build_image(flavor, no_cache);
}

pub fn dispatch(cwd: &Path, flavor: &str, args: Vec<String>) -> i32 {
    let Some(agent) = FlavorConfig::load_or_default(flavor).agent else {
        die(format!(
            "flavor '{flavor}' is not an agent (no [agent] in its config.toml)"
        ));
    };

    let mut inv = Invocation::default();
    let rest = extract_sbx_flags(args, &agent, &mut inv);

    match rest.first().map(String::as_str) {
        Some("shell") | Some("bash") => inv.shell = true,
        Some("build") => {
            build(flavor, false);
            return 0;
        }
        Some("rebuild") => {
            build(flavor, true);
            return 0;
        }
        Some("profile") if agent.profiles => {
            profiles::dispatch(flavor, cwd, &rest);
            return 0;
        }
        _ => inv.args = rest,
    }

    launch(cwd, flavor, &agent, inv)
}

fn launch(cwd: &Path, flavor: &str, agent: &Agent, inv: Invocation) -> i32 {
    docker::ensure_ssh_agent_ready(cwd);
    image_exists_or_build(flavor);

    let chome = container_home();
    let entry = if inv.shell {
        vec!["/bin/bash".into()]
    } else {
        build_entry(flavor, agent, &inv.args, &inv, cwd)
    };

    let extra_mounts = mounts::resolve(cwd, &chome, &inv.mounts, Some(flavor));

    let extra_host_args = profiles::mount_args(cwd, flavor, agent, inv.profile.as_deref(), &chome);

    let mount_docker_socket = inv.docker
        || std::env::var("SBX_DOCKER")
            .map(|v| v == "1")
            .unwrap_or(false);

    let image = image_name(flavor);
    let mut extra_env = vec![
        ("SBX_PROJECT".into(), project_name(cwd)),
        ("SBX_PROJECT_BASE".into(), project_base_name(cwd)),
        (
            "SBX_WORKTREE".into(),
            worktree_suffix(cwd).unwrap_or_default(),
        ),
    ];
    for key in &agent.forward_env {
        if let Ok(val) = std::env::var(key)
            && !val.is_empty()
        {
            extra_env.push((key.clone(), val));
        }
    }

    let spec = RunSpec {
        image: &image,
        flavor,
        project_root: cwd,
        entry,
        network: Network::Bridge,
        use_hostname: true,
        publish_ports: PortSpec::default(),
        extra_host_args,
        extra_mounts,
        container_home: chome,
        labels: Vec::new(),
        mount_docker_socket,
        extra_env,
        force_gpg: inv.gpg,
    };
    docker::run_container(spec)
}

fn build_entry(
    flavor: &str,
    agent: &Agent,
    args: &[String],
    inv: &Invocation,
    cwd: &Path,
) -> Vec<String> {
    let mut v = vec![agent.binary(flavor).to_string()];

    if !inv.safe && !agent.autonomy.is_empty() && !agent.already_autonomous(args) {
        v.extend(agent.autonomy.iter().cloned());
    }

    if agent.remote_control {
        let rc_on = !inv.no_rc
            && (inv.rc
                || std::env::var("SBX_REMOTE_CONTROL")
                    .map(|val| val == "1")
                    .unwrap_or(false));
        let rc_already = args.iter().any(|a| a == "--remote-control" || a == "--rc");
        if rc_on && !rc_already {
            let name = format!("{}-{}", project_name(cwd), std::process::id());
            v.push("--remote-control".into());
            v.push(name);
        }
    }

    v.extend(args.iter().cloned());
    v
}

fn extract_sbx_flags(args: Vec<String>, agent: &Agent, inv: &mut Invocation) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut iter = args.into_iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--shell" => inv.shell = true,
            "--docker" => inv.docker = true,
            "--git" | "--gpg" => inv.gpg = true,
            "--safe" | "-s" => inv.safe = true,
            "--mount" | "-m" => match iter.next() {
                Some(v) => inv.mounts.push(v),
                None => die("--mount requires a value"),
            },
            "--profile" | "-p" if agent.profiles => match iter.next() {
                Some(v) => inv.profile = Some(v),
                None => die("--profile requires a value"),
            },
            "--rc" if agent.remote_control => inv.rc = true,
            "--no-rc" if agent.remote_control => inv.no_rc = true,
            _ => {
                if let Some(v) = a.strip_prefix("--mount=") {
                    inv.mounts.push(v.to_string());
                } else if agent.profiles
                    && let Some(v) = a.strip_prefix("--profile=")
                {
                    inv.profile = Some(v.to_string());
                } else {
                    out.push(a);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn copilot_agent() -> Agent {
        Agent {
            bin: Some("copilot".into()),
            persist: vec![".copilot".into()],
            autonomy: vec!["--allow-all".into()],
            autonomy_detect: vec!["--yolo".into()],
            forward_env: vec!["GH_TOKEN".into()],
            ..Default::default()
        }
    }

    #[test]
    fn entry_injects_autonomy_then_args() {
        let agent = copilot_agent();
        let inv = Invocation::default();
        let entry = build_entry("copilot", &agent, &["chat".into()], &inv, Path::new("/tmp"));
        assert_eq!(entry, vec!["copilot", "--allow-all", "chat"]);
    }

    #[test]
    fn entry_respects_safe_flag() {
        let agent = copilot_agent();
        let inv = Invocation {
            safe: true,
            ..Default::default()
        };
        let entry = build_entry("copilot", &agent, &[], &inv, Path::new("/tmp"));
        assert_eq!(entry, vec!["copilot"]);
    }

    #[test]
    fn entry_skips_autonomy_when_already_present() {
        let agent = copilot_agent();
        let inv = Invocation::default();
        let entry = build_entry(
            "copilot",
            &agent,
            &["--yolo".into()],
            &inv,
            Path::new("/tmp"),
        );
        assert_eq!(entry, vec!["copilot", "--yolo"]);
    }

    #[test]
    fn entry_uses_flavor_name_when_bin_unset() {
        let agent = Agent::default();
        let inv = Invocation::default();
        let entry = build_entry(
            "opencode",
            &agent,
            &["auth".into()],
            &inv,
            Path::new("/tmp"),
        );
        assert_eq!(entry, vec!["opencode", "auth"]);
    }

    #[test]
    fn extract_pulls_sbx_flags_and_keeps_passthrough() {
        let agent = copilot_agent();
        let mut inv = Invocation::default();
        let rest = extract_sbx_flags(
            vec![
                "--docker".into(),
                "--mount".into(),
                "~/x".into(),
                "--safe".into(),
                "chat".into(),
                "--allow-all".into(),
            ],
            &agent,
            &mut inv,
        );
        assert!(inv.docker);
        assert!(inv.safe);
        assert_eq!(inv.mounts, vec!["~/x"]);
        assert_eq!(rest, vec!["chat", "--allow-all"]);
    }

    #[test]
    fn extract_leaves_profile_and_rc_for_non_claude_agents() {
        // copilot doesn't enable profiles/remote-control, so these pass through.
        let agent = copilot_agent();
        let mut inv = Invocation::default();
        let rest = extract_sbx_flags(
            vec!["--profile".into(), "work".into(), "--rc".into()],
            &agent,
            &mut inv,
        );
        assert!(inv.profile.is_none());
        assert!(!inv.rc);
        assert_eq!(rest, vec!["--profile", "work", "--rc"]);
    }

    #[test]
    fn extract_consumes_profile_and_rc_for_claude() {
        let agent = Agent {
            profiles: true,
            remote_control: true,
            autonomy: vec!["--dangerously-skip-permissions".into()],
            ..Default::default()
        };
        let mut inv = Invocation::default();
        let rest = extract_sbx_flags(
            vec![
                "--profile=work".into(),
                "--rc".into(),
                "-p".into(),
                "other".into(),
            ],
            &agent,
            &mut inv,
        );
        assert!(inv.rc);
        assert_eq!(inv.profile.as_deref(), Some("other"));
        assert!(rest.is_empty());
    }

    #[test]
    fn extract_handles_shell_flag() {
        let agent = Agent::default();
        let mut inv = Invocation::default();
        let rest = extract_sbx_flags(vec!["--shell".into()], &agent, &mut inv);
        assert!(inv.shell);
        assert!(rest.is_empty());
    }

    #[test]
    fn extract_handles_safe_short_flag() {
        let agent = copilot_agent();
        let mut inv = Invocation::default();
        let rest = extract_sbx_flags(vec!["-s".into(), "chat".into()], &agent, &mut inv);
        assert!(inv.safe);
        assert_eq!(rest, vec!["chat"]);
    }
}
