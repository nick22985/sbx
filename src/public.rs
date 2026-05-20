use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::Config;
use crate::flavor::{nix_gid, nix_uid};
use crate::util::{config_dir, die, log};

pub const SIDECAR: &str = "sbx-public";
pub const TUNNEL_NAME: &str = "sbx-public";
pub const IMAGE: &str = "cloudflare/cloudflared:latest";

pub fn cf_dir() -> PathBuf {
    config_dir().join("cloudflared")
}
pub fn cert_pem() -> PathBuf {
    cf_dir().join("cert.pem")
}
pub fn credentials_file() -> PathBuf {
    cf_dir().join("credentials.json")
}
pub fn config_yml() -> PathBuf {
    cf_dir().join("config.yml")
}
pub fn projects_dir() -> PathBuf {
    cf_dir().join("projects")
}
pub fn logged_in() -> bool {
    cert_pem().is_file()
}
pub fn tunnel_exists() -> bool {
    credentials_file().is_file()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicRoute {
    pub hostname: String,
    pub port: u16,
}

impl crate::project::WorktreeAdjustable for PublicRoute {
    fn hostname_mut(&mut self) -> &mut String {
        &mut self.hostname
    }
    fn port_mut(&mut self) -> &mut u16 {
        &mut self.port
    }
}

pub fn read_public(root: &Path) -> Vec<PublicRoute> {
    let mut routes: Vec<PublicRoute> = Config::load_or_default(root)
        .public
        .into_iter()
        .map(|(hostname, port)| PublicRoute { hostname, port })
        .collect();
    crate::project::apply_worktree_remap(root, &mut routes);
    routes
}

fn user_arg() -> String {
    format!("{}:{}", nix_uid(), nix_gid())
}

fn add_mount(cmd: &mut Command, cf: &Path) {
    cmd.args(["-v", &format!("{}:/etc/cloudflared", cf.display())]);
    cmd.args(["--user", &user_arg()]);
    cmd.args(["-w", "/etc/cloudflared"]);
    cmd.args(["-e", "HOME=/etc/cloudflared"]);
    cmd.args(["-e", "TUNNEL_ORIGIN_CERT=/etc/cloudflared/cert.pem"]);
}

pub fn login() -> Result<(), String> {
    let cf = cf_dir();
    std::fs::create_dir_all(&cf).map_err(|e| format!("create {}: {e}", cf.display()))?;
    log("running 'cloudflared tunnel login' — open the URL printed below in a browser");
    let mut cmd = Command::new("docker");
    cmd.args(["run", "--rm", "-it"]);
    add_mount(&mut cmd, &cf);
    cmd.args([
        IMAGE,
        "tunnel",
        "--origincert",
        "/etc/cloudflared/cert.pem",
        "login",
    ]);
    let status = cmd.status().map_err(|e| format!("docker: {e}"))?;
    if !status.success() {
        return Err("cloudflared login failed".into());
    }
    let nested = cf.join(".cloudflared").join("cert.pem");
    if !cert_pem().is_file() && nested.is_file() {
        std::fs::rename(&nested, cert_pem())
            .map_err(|e| format!("move {}: {e}", nested.display()))?;
        let _ = std::fs::remove_dir(cf.join(".cloudflared"));
    }
    if !cert_pem().is_file() {
        return Err(format!(
            "cert.pem not found at {} after login",
            cert_pem().display()
        ));
    }
    log(format!("cert.pem written to {}", cert_pem().display()));
    Ok(())
}

pub fn ensure_tunnel() -> Result<(), String> {
    if tunnel_exists() {
        return Ok(());
    }
    if !logged_in() {
        return Err("not logged in. run: sbx public login".into());
    }
    let cf = cf_dir();
    log(format!("creating cloudflare tunnel '{TUNNEL_NAME}'"));
    let mut cmd = Command::new("docker");
    cmd.args(["run", "--rm"]);
    add_mount(&mut cmd, &cf);
    cmd.args([
        IMAGE,
        "tunnel",
        "--origincert",
        "/etc/cloudflared/cert.pem",
        "create",
        "--credentials-file",
        "/etc/cloudflared/credentials.json",
        TUNNEL_NAME,
    ]);
    let status = cmd.status().map_err(|e| format!("docker: {e}"))?;
    if !status.success() {
        return Err("cloudflared tunnel create failed".into());
    }
    if !credentials_file().is_file() {
        return Err(format!(
            "credentials.json not written to {}",
            credentials_file().display()
        ));
    }
    Ok(())
}

pub fn ensure_dns_route(hostname: &str) -> Result<(), String> {
    let cf = cf_dir();
    let mut cmd = Command::new("docker");
    cmd.args(["run", "--rm"]);
    add_mount(&mut cmd, &cf);
    cmd.args([
        IMAGE,
        "tunnel",
        "--origincert",
        "/etc/cloudflared/cert.pem",
        "route",
        "dns",
        "--overwrite-dns",
        TUNNEL_NAME,
        hostname,
    ]);
    let status = cmd.status().map_err(|e| format!("docker: {e}"))?;
    if !status.success() {
        return Err(format!("cloudflared route dns failed for {hostname}"));
    }
    Ok(())
}

pub fn write_project_fragment(pname: &str, hostnames: &[String]) -> Result<(), String> {
    crate::fragments::write(&projects_dir(), pname, hostnames)
}

pub fn remove_project_fragment(pname: &str) {
    crate::fragments::remove(&projects_dir(), pname);
}

pub fn touch_project_fragment(pname: &str) {
    crate::fragments::touch(&projects_dir(), pname);
}

pub fn delete_project_dns_routes(pname: &str) {
    if crate::cloudflare::api_token().is_none() {
        return;
    }
    let path = projects_dir().join(pname);
    let Ok(body) = std::fs::read_to_string(&path) else {
        return;
    };
    for host in crate::util::config_lines(&body) {
        if let Err(e) = crate::cloudflare::delete_dns_route(host) {
            log(format!("warn: cf delete {host}: {e}"));
        }
    }
}

pub fn merged_hostnames() -> Vec<String> {
    crate::fragments::merged(&projects_dir())
}

pub fn render_config_yml() -> Result<(), String> {
    let hosts = merged_hostnames();
    let mut yml = String::new();
    yml.push_str(&format!("tunnel: {TUNNEL_NAME}\n"));
    yml.push_str("credentials-file: /etc/cloudflared/credentials.json\n");
    yml.push_str("ingress:\n");
    for h in &hosts {
        yml.push_str(&format!(
            "  - hostname: {h}\n    service: http://sbx-proxy:80\n"
        ));
    }
    yml.push_str("  - service: http_status:404\n");
    std::fs::create_dir_all(cf_dir()).ok();
    std::fs::write(config_yml(), yml).map_err(|e| format!("write config.yml: {e}"))
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

pub fn start_sidecar() {
    if merged_hostnames().is_empty() {
        return;
    }
    crate::proxy::ensure_network();
    let cf = cf_dir();
    if sidecar_running() {
        return;
    }
    if sidecar_exists() {
        force_stop();
    }
    log(format!("starting cloudflared sidecar: {SIDECAR}"));
    let mut cmd = Command::new("docker");
    cmd.args([
        "run",
        "-d",
        "--name",
        SIDECAR,
        "--network",
        crate::proxy::NETWORK,
        "--restart",
        "unless-stopped",
        "-v",
        &format!("{}:/etc/cloudflared:ro", cf.display()),
        "--user",
        &user_arg(),
        IMAGE,
        "tunnel",
        "--no-autoupdate",
        "--config",
        "/etc/cloudflared/config.yml",
        "run",
    ]);
    match cmd.status() {
        Ok(s) if s.success() => log(format!("cloudflared sidecar up: {SIDECAR}")),
        _ => die("failed to start cloudflared sidecar"),
    }
}

pub fn apply_config() {
    reconcile_orphan_fragments();
    if merged_hostnames().is_empty() {
        force_stop();
        let _ = std::fs::remove_file(config_yml());
        return;
    }
    let _ = render_config_yml();
    if sidecar_running() {
        let _ = Command::new("docker")
            .args(["restart", SIDECAR])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else {
        start_sidecar();
    }
}

pub fn stop_sidecar_if_idle() {
    if merged_hostnames().is_empty() {
        force_stop();
        let _ = std::fs::remove_file(config_yml());
    }
}

const ORPHAN_GRACE_SECS: u64 = 300;

pub fn reconcile_orphan_fragments() {
    let Some(running) = running_sbx_project_names() else {
        return;
    };
    let dir = projects_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(String::from) else {
            continue;
        };
        if running.contains(&name) {
            continue;
        }
        if let Ok(meta) = entry.metadata()
            && let Ok(mtime) = meta.modified()
            && let Ok(age) = now.duration_since(mtime)
            && age.as_secs() < ORPHAN_GRACE_SECS
        {
            continue;
        }
        delete_project_dns_routes(&name);
        let _ = std::fs::remove_file(entry.path());
        log(format!(
            "public: pruned orphan fragment for {name} (no live container)"
        ));
    }
}

fn running_sbx_project_names() -> Option<HashSet<String>> {
    let out = Command::new("docker")
        .args(["ps", "--filter", "name=^sbx-", "--format", "{{.Names}}"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let names: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let mut set: HashSet<String> = HashSet::new();
    if names.is_empty() {
        return Some(set);
    }
    let mut args: Vec<String> = vec![
        "inspect".into(),
        "--format".into(),
        "{{range .Config.Env}}{{println .}}{{end}}".into(),
    ];
    args.extend(names);
    let out = Command::new("docker").args(&args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(rest) = line.strip_prefix("SBX_PROJECT=") {
            let v = rest.trim();
            if !v.is_empty() {
                set.insert(v.to_string());
            }
        }
    }
    Some(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_config_yml_fragment() {
        let hosts = vec!["a.example.com".to_string(), "b.example.com".to_string()];
        let mut yml = String::new();
        yml.push_str(&format!("tunnel: {TUNNEL_NAME}\n"));
        yml.push_str("credentials-file: /etc/cloudflared/credentials.json\n");
        yml.push_str("ingress:\n");
        for h in &hosts {
            yml.push_str(&format!(
                "  - hostname: {h}\n    service: http://sbx-proxy:80\n"
            ));
        }
        yml.push_str("  - service: http_status:404\n");
        assert!(yml.contains("hostname: a.example.com"));
        assert!(yml.contains("http://sbx-proxy:80"));
        assert!(yml.ends_with("http_status:404\n"));
    }
}
