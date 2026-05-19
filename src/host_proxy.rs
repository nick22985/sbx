use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::Config;
use crate::util::{config_dir, log};

pub const SIDECAR: &str = "sbx-host-proxy";
pub const PORT: u16 = 8118;
const IMAGE: &str = "kalaksi/tinyproxy:latest";

pub fn is_enabled(project_root: &Path) -> bool {
    Config::load_or_default(project_root).host_proxy.enabled
}

pub fn data_dir() -> PathBuf {
    config_dir().join("host-proxy")
}

pub fn config_path() -> PathBuf {
    data_dir().join("tinyproxy.conf")
}

pub fn filter_path() -> PathBuf {
    data_dir().join("filter")
}

pub fn projects_dir() -> PathBuf {
    data_dir().join("projects")
}

pub fn read_allowed_hosts(project_root: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen = BTreeSet::new();
    for h in Config::load_or_default(project_root).host_proxy.allow {
        if seen.insert(h.clone()) {
            out.push(h);
        }
    }
    out
}

pub fn write_project_fragment(pname: &str, hosts: &[String]) -> Result<(), String> {
    crate::fragments::write(&projects_dir(), pname, hosts)
}

pub fn remove_project_fragment(pname: &str) {
    crate::fragments::remove(&projects_dir(), pname);
}

pub fn merged_allowlist() -> Vec<String> {
    crate::fragments::merged(&projects_dir())
}

/// Translate a hostname pattern to a tinyproxy filter regex.
///
/// - `foo.com`     → `^foo\.com$`           (exact)
/// - `*.foo.com`   → `^.*\.foo\.com$`        (subdomains only)
/// - `*foo.com`    → `^.*foo\.com$`          (general wildcard)
pub fn hostname_to_regex(host: &str) -> String {
    let mut out = String::from("^");
    for ch in host.chars() {
        match ch {
            '*' => out.push_str(".*"),
            '.' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\\' | '^' | '$' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out.push('$');
    out
}

pub fn render_filter(hosts: &[String]) -> String {
    let mut s = String::new();
    for h in hosts {
        s.push_str(&hostname_to_regex(h));
        s.push('\n');
    }
    s
}

pub fn render_config(filter_enabled: bool) -> String {
    let mut s = String::new();
    s.push_str("User tinyproxy\n");
    s.push_str("Group tinyproxy\n");
    s.push_str(&format!("Port {PORT}\n"));
    s.push_str("Listen 0.0.0.0\n");
    s.push_str("Timeout 600\n");
    s.push_str("MaxClients 100\n");
    s.push_str("LogLevel Info\n");
    s.push_str("DisableViaHeader Yes\n");
    s.push_str("Allow 127.0.0.0/8\n");
    s.push_str("Allow ::1\n");
    s.push_str("Allow 10.0.0.0/8\n");
    s.push_str("Allow 172.16.0.0/12\n");
    s.push_str("Allow 192.168.0.0/16\n");
    s.push_str("ConnectPort 443\n");
    s.push_str("ConnectPort 563\n");
    if filter_enabled {
        s.push_str("Filter \"/etc/tinyproxy/filter\"\n");
        s.push_str("FilterDefaultDeny Yes\n");
        s.push_str("FilterExtended Yes\n");
        s.push_str("FilterCaseSensitive No\n");
        s.push_str("FilterURLs Off\n");
    }
    s
}

pub fn write_files() -> Result<(PathBuf, PathBuf), String> {
    let dir = data_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let hosts = merged_allowlist();
    let filter_enabled = !hosts.is_empty();

    let filter_p = filter_path();
    std::fs::write(&filter_p, render_filter(&hosts))
        .map_err(|e| format!("write {}: {e}", filter_p.display()))?;

    let conf_p = config_path();
    std::fs::write(&conf_p, render_config(filter_enabled))
        .map_err(|e| format!("write {}: {e}", conf_p.display()))?;
    Ok((conf_p, filter_p))
}

pub fn sidecar_running() -> bool {
    crate::docker::container_exists(SIDECAR, false)
}

pub fn sidecar_exists() -> bool {
    crate::docker::container_exists(SIDECAR, true)
}

pub fn force_stop() -> bool {
    crate::docker::stop_if_present(SIDECAR)
}

pub fn reload() -> bool {
    if !sidecar_running() {
        return false;
    }
    let out = Command::new("docker")
        .args(["kill", "--signal=HUP", SIDECAR])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    matches!(out, Ok(s) if s.success())
}

pub fn start_sidecar() -> Result<(), String> {
    let (conf, filter) = write_files()?;
    if sidecar_running() {
        return Ok(());
    }
    if sidecar_exists() {
        force_stop();
    }
    log(format!(
        "starting host-proxy sidecar: {SIDECAR} (tinyproxy on :{PORT})"
    ));
    let mut cmd = Command::new("docker");
    cmd.args([
        "run",
        "-d",
        "--name",
        SIDECAR,
        "--network",
        "host",
        "--restart",
        "unless-stopped",
        "-v",
        &format!("{}:/etc/tinyproxy/tinyproxy.conf:ro", conf.display()),
        "-v",
        &format!("{}:/etc/tinyproxy/filter:ro", filter.display()),
        IMAGE,
    ]);
    let out = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).output();
    match out {
        Ok(o) if o.status.success() => {
            log(format!("host-proxy up: http://host.docker.internal:{PORT}"));
            Ok(())
        }
        Ok(o) => {
            for line in String::from_utf8_lossy(&o.stderr).lines() {
                log(format!("  docker: {line}"));
            }
            Err("failed to start host-proxy sidecar".into())
        }
        Err(e) => Err(format!("failed to spawn docker: {e}")),
    }
}

/// Re-render conf+filter from disk state and SIGHUP a running sidecar.
/// Starts the sidecar if it isn't running but a marker is set somewhere.
pub fn apply_config() -> Result<(), String> {
    let _ = write_files()?;
    if sidecar_running() {
        if reload() {
            let hosts = merged_allowlist();
            if hosts.is_empty() {
                log("host-proxy reloaded (unrestricted)");
            } else {
                log(format!(
                    "host-proxy reloaded: allowlist now {} entr{}",
                    hosts.len(),
                    if hosts.len() == 1 { "y" } else { "ies" }
                ));
            }
        } else {
            log("host-proxy reload (SIGHUP) failed");
        }
    }
    Ok(())
}

pub fn stop_sidecar_if_idle() {
    if !sidecar_running() {
        return;
    }
    if any_running_sandbox_needs_proxy() {
        return;
    }
    log(format!("stopping host-proxy sidecar: {SIDECAR}"));
    force_stop();
}

fn any_running_sandbox_needs_proxy() -> bool {
    let Ok(out) = Command::new("docker")
        .args(["ps", "--filter", "name=^sbx-", "--format", "{{.Names}}"])
        .output()
    else {
        return false;
    };
    for name in String::from_utf8_lossy(&out.stdout).lines() {
        let n = name.trim();
        if n.is_empty() || n == SIDECAR {
            continue;
        }
        let envs = Command::new("docker")
            .args([
                "inspect",
                "--format",
                "{{range .Config.Env}}{{.}}\n{{end}}",
                n,
            ])
            .output();
        if let Ok(o) = envs {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                if line.starts_with("https_proxy=") || line.starts_with("HTTPS_PROXY=") {
                    return true;
                }
            }
        }
    }
    false
}

/// Docker `-e` args injecting standard proxy env vars pointing at the sidecar.
pub fn proxy_env_args() -> Vec<String> {
    let url = format!("http://host.docker.internal:{PORT}");
    let no_proxy = "localhost,127.0.0.1,::1,host.docker.internal";
    let pairs = [
        ("http_proxy", url.as_str()),
        ("HTTP_PROXY", url.as_str()),
        ("https_proxy", url.as_str()),
        ("HTTPS_PROXY", url.as_str()),
        ("no_proxy", no_proxy),
        ("NO_PROXY", no_proxy),
    ];
    let mut out = Vec::with_capacity(pairs.len() * 2);
    for (k, v) in pairs {
        out.push("-e".into());
        out.push(format!("{k}={v}"));
    }
    out
}

/// Read the project's allowlist, add `host` if not already present, write back.
pub fn add_allowed_host(project_root: &Path, host: &str) -> Result<bool, String> {
    let hosts = read_allowed_hosts(project_root);
    if hosts.iter().any(|h| h == host) {
        return Ok(false);
    }
    Config::edit(project_root, |c| {
        if !c.host_proxy.allow.iter().any(|h| h == host) {
            c.host_proxy.allow.push(host.to_string());
        }
    })
    .map_err(|e| format!("write config.toml: {e}"))?;
    Ok(true)
}

/// Read the project's allowlist, remove `host` if present, write back.
pub fn remove_allowed_host(project_root: &Path, host: &str) -> Result<bool, String> {
    let hosts = read_allowed_hosts(project_root);
    if !hosts.iter().any(|h| h == host) {
        return Ok(false);
    }
    Config::edit(project_root, |c| {
        c.host_proxy.allow.retain(|h| h != host);
    })
    .map_err(|e| format!("write config.toml: {e}"))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_baseline_no_filter() {
        let s = render_config(false);
        assert!(s.contains(&format!("Port {PORT}")));
        assert!(s.contains("Listen 0.0.0.0"));
        assert!(s.contains("Allow 172.16.0.0/12"));
        assert!(s.contains("ConnectPort 443"));
        assert!(!s.contains("FilterDefaultDeny"));
        assert!(!s.contains("Filter \""));
    }

    #[test]
    fn config_with_filter_includes_default_deny() {
        let s = render_config(true);
        assert!(s.contains("Filter \"/etc/tinyproxy/filter\""));
        assert!(s.contains("FilterDefaultDeny Yes"));
        assert!(s.contains("FilterURLs Off"));
    }

    #[test]
    fn env_args_include_https_proxy() {
        let args = proxy_env_args();
        let joined = args.join(" ");
        assert!(joined.contains("https_proxy=http://host.docker.internal:8118"));
        assert!(joined.contains("HTTPS_PROXY=http://host.docker.internal:8118"));
        assert!(joined.contains("no_proxy="));
    }

    #[test]
    fn regex_escapes_dots_and_anchors() {
        assert_eq!(
            hostname_to_regex("repo.example.com"),
            r"^repo\.example\.com$"
        );
    }

    #[test]
    fn regex_expands_star_wildcard() {
        assert_eq!(hostname_to_regex("*.maven.org"), r"^.*\.maven\.org$");
    }

    #[test]
    fn render_filter_one_line_per_host() {
        let body = render_filter(&["foo.com".into(), "*.bar.org".into()]);
        assert_eq!(body, "^foo\\.com$\n^.*\\.bar\\.org$\n");
    }
}
