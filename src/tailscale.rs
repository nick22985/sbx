use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::project::project_name;
use crate::util::{die, log};

pub const AUTHKEY_ENV_BASE: &str = "SBX_TAILSCALE_AUTHKEY";
const IMAGE: &str = "tailscale/tailscale:latest";

pub fn authkey_env(profile: Option<&str>) -> String {
    match profile {
        None => AUTHKEY_ENV_BASE.to_string(),
        Some(name) => format!("{AUTHKEY_ENV_BASE}_{}", normalize_env_suffix(name)),
    }
}

fn normalize_env_suffix(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' => c.to_ascii_uppercase(),
            _ => '_',
        })
        .collect()
}

pub fn is_valid_profile_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub fn sidecar_name(pname: &str, profile: Option<&str>) -> String {
    match profile {
        None => format!("sbx-tailscale-{pname}"),
        Some(p) => format!("sbx-tailscale-{pname}-{p}"),
    }
}

pub fn state_volume(pname: &str, profile: Option<&str>) -> String {
    match profile {
        None => format!("sbx-tailscale-{pname}-state"),
        Some(p) => format!("sbx-tailscale-{pname}-{p}-state"),
    }
}

pub fn sidecar_running(name: &str) -> bool {
    container_exists(name, false)
}

pub fn sidecar_exists(name: &str) -> bool {
    container_exists(name, true)
}

fn container_exists(name: &str, include_stopped: bool) -> bool {
    let mut cmd = Command::new("docker");
    cmd.arg("ps");
    if include_stopped {
        cmd.arg("-a");
    }
    cmd.args([
        "--filter",
        &format!("name=^{name}$"),
        "--format",
        "{{.Names}}",
    ]);
    let Ok(out) = cmd.output() else { return false };
    !String::from_utf8_lossy(&out.stdout).trim().is_empty()
}

pub fn sidecar_attached_count(sidecar: &str) -> u32 {
    crate::docker::netns_attached_count(sidecar)
}

pub fn start_sidecar(
    project_root: &Path,
    share_netns: Option<&str>,
    profile: Option<&str>,
    publish_args: &[String],
) -> String {
    let pname = project_name(project_root);
    let sidecar = sidecar_name(&pname, profile);

    if sidecar_running(&sidecar) {
        let attached = sidecar_attached_count(&sidecar);
        log(format!(
            "reusing tailscale sidecar: {sidecar} (serving {attached} container(s))"
        ));
        return sidecar;
    }
    if sidecar_exists(&sidecar) {
        force_rm(&sidecar);
    }

    let env_name = authkey_env(profile);
    let authkey = std::env::var(&env_name).unwrap_or_default();
    let interactive = authkey.is_empty();
    if interactive {
        let auth_cmd = match profile {
            None => "sbx net tailscale auth".to_string(),
            Some(p) => format!("sbx net tailscale auth {p}"),
        };
        log(format!(
            "{env_name} is unset — using interactive login. (run '{auth_cmd}' to skip this next time)"
        ));
    }

    let hostname = match profile {
        None => format!("sbx-{pname}"),
        Some(p) => format!("sbx-{pname}-{p}"),
    };
    let state = state_volume(&pname, profile);

    log(format!("starting tailscale sidecar: {sidecar}"));
    if let Some(n) = share_netns {
        log(format!("  attaching to netns of: {n}"));
    }

    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", &sidecar]);
    cmd.args([
        "--cap-add=NET_ADMIN",
        "--device=/dev/net/tun",
        "-e",
        &format!("TS_HOSTNAME={hostname}"),
        "-e",
        "TS_STATE_DIR=/var/lib/tailscale",
        "-e",
        "TS_USERSPACE=false",
    ]);
    if !authkey.is_empty() {
        cmd.args(["-e", &format!("TS_AUTHKEY={authkey}")]);
    }
    if let Ok(extra) = std::env::var("SBX_TAILSCALE_EXTRA_ARGS")
        && !extra.is_empty()
    {
        cmd.args(["-e", &format!("TS_EXTRA_ARGS={extra}")]);
    }
    cmd.args(["-v", &format!("{state}:/var/lib/tailscale")]);
    if let Some(owner) = share_netns {
        cmd.args(["--network", &format!("container:{owner}")]);
    } else {
        cmd.arg("--add-host=host.docker.internal:host-gateway");
        for a in publish_args {
            cmd.arg(a);
        }
    }
    cmd.arg(IMAGE);
    let status = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).status();
    if status.map(|s| !s.success()).unwrap_or(true) {
        die("failed to start tailscale sidecar");
    }

    let wait_secs = if interactive { 1800 } else { 60 };
    let mut login_url_shown = false;
    for i in 0..wait_secs {
        if !sidecar_running(&sidecar) {
            log("tailscale sidecar exited unexpectedly; logs:");
            print_logs(&sidecar);
            log(format!(
                "(container kept for inspection: docker logs {sidecar}; clean with: docker rm {sidecar})"
            ));
            die("tailscale sidecar failed to start");
        }
        let ok = Command::new("docker")
            .args(["exec", &sidecar, "tailscale", "status", "--self=false"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            log(format!("tailscale sidecar up: {sidecar}"));
            return sidecar;
        }
        if interactive
            && !login_url_shown
            && i >= 2
            && let Some(url) = scan_login_url(&sidecar)
        {
            log(
                "tailscale needs interactive auth. Open this URL or sign in via the Tailscale app:",
            );
            log(format!("  {url}"));
            log(format!(
                "(waiting up to {wait_secs}s — interrupt with Ctrl-C if you want to abort)"
            ));
            login_url_shown = true;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    log(format!(
        "tailscale sidecar didn't come up in {wait_secs}s; logs:"
    ));
    print_logs(&sidecar);
    force_rm(&sidecar);
    die(format!("tailscale sidecar didn't come up in {wait_secs}s"));
}

fn scan_login_url(name: &str) -> Option<String> {
    let out = Command::new("docker")
        .args(["logs", "--tail", "200", name])
        .output()
        .ok()?;
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for line in text.lines() {
        for token in line.split_whitespace() {
            if token.starts_with("https://login.tailscale.com/")
                || token.starts_with("https://controlplane.tailscale.com/")
            {
                return Some(token.trim_end_matches([',', '.', ')']).to_string());
            }
        }
    }
    None
}

pub fn stop_sidecar(sidecar: &str) {
    if !sidecar_exists(sidecar) {
        return;
    }
    log(format!("stopping tailscale sidecar: {sidecar}"));
    force_rm(sidecar);
}

pub fn stop_sidecar_if_idle(sidecar: &str) {
    if !sidecar_running(sidecar) {
        return;
    }
    let n = sidecar_attached_count(sidecar);
    if n > 0 {
        log(format!(
            "tailscale sidecar still has {n} attached container(s); leaving {sidecar} up"
        ));
        return;
    }
    stop_sidecar(sidecar);
}

fn force_rm(name: &str) {
    let _ = Command::new("docker")
        .args(["rm", "-f", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn print_logs(name: &str) {
    let _ = Command::new("docker")
        .args(["logs", "--tail", "80", name])
        .status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_env_suffix_uppercases_alphanumerics() {
        assert_eq!(normalize_env_suffix("work"), "WORK");
        assert_eq!(normalize_env_suffix("test2"), "TEST2");
    }

    #[test]
    fn normalize_env_suffix_replaces_non_alphanumerics_with_underscore() {
        assert_eq!(normalize_env_suffix("my-profile.2"), "MY_PROFILE_2");
        assert_eq!(normalize_env_suffix("a b"), "A_B");
    }

    #[test]
    fn authkey_env_uses_base_when_no_profile() {
        assert_eq!(authkey_env(None), AUTHKEY_ENV_BASE);
    }

    #[test]
    fn authkey_env_appends_normalized_suffix_for_profile() {
        assert_eq!(
            authkey_env(Some("my-work")),
            format!("{AUTHKEY_ENV_BASE}_MY_WORK")
        );
    }

    #[test]
    fn is_valid_profile_name_accepts_lowercase_digits_dash_underscore() {
        assert!(is_valid_profile_name("work"));
        assert!(is_valid_profile_name("my-profile_2"));
    }

    #[test]
    fn is_valid_profile_name_rejects_empty_uppercase_and_punctuation() {
        assert!(!is_valid_profile_name(""));
        assert!(!is_valid_profile_name("Work"));
        assert!(!is_valid_profile_name("my.profile"));
        assert!(!is_valid_profile_name("my profile"));
    }

    #[test]
    fn sidecar_name_format() {
        assert_eq!(sidecar_name("proj", None), "sbx-tailscale-proj");
        assert_eq!(
            sidecar_name("proj", Some("work")),
            "sbx-tailscale-proj-work"
        );
    }

    #[test]
    fn state_volume_format() {
        assert_eq!(state_volume("proj", None), "sbx-tailscale-proj-state");
        assert_eq!(
            state_volume("proj", Some("work")),
            "sbx-tailscale-proj-work-state"
        );
    }
}
