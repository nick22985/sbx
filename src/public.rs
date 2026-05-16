use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::flavor::{nix_gid, nix_uid};
use crate::project::sbx_file;
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
    let p = sbx_file(root, "public");
    let body = std::fs::read_to_string(&p).unwrap_or_default();
    let mut routes = parse_public(&body);
    crate::project::apply_worktree_remap(root, &mut routes);
    routes
}

pub fn parse_public(body: &str) -> Vec<PublicRoute> {
    let mut out = Vec::new();
    for line in crate::util::config_lines(body) {
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        let host = lhs.trim();
        let port = rhs.trim();
        if host.is_empty() {
            continue;
        }
        let Ok(p) = port.parse::<u16>() else { continue };
        out.push(PublicRoute {
            hostname: host.to_string(),
            port: p,
        });
    }
    out
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
    if merged_hostnames().is_empty() {
        force_stop();
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_basic() {
        let r = parse_public(
            "app.example.com = 8080\napi.example.com=1337  # comment\n# full\n\nbad\n=80\n",
        );
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].hostname, "app.example.com");
        assert_eq!(r[0].port, 8080);
        assert_eq!(r[1].hostname, "api.example.com");
        assert_eq!(r[1].port, 1337);
    }

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
