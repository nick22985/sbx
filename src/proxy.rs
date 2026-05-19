use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::util::{config_dir, die, log, sanitize_tag};

pub const NETWORK: &str = "sbx-proxy-net";
pub const SIDECAR: &str = "sbx-proxy";
pub const DASHBOARD_HOST: &str = "traefik.sbx.localhost";
const IMAGE: &str = "traefik:v3";
const DYNAMIC_MOUNT: &str = "/etc/traefik/dynamic";
const DASHBOARD_FILE: &str = "_sbx-dashboard.yaml";
const CERT_RESOLVER: &str = "cloudflare";
const CERT_FILE: &str = "cert.pem";
const KEY_FILE: &str = "key.pem";
const TLS_DEFAULTS_FILE: &str = "_sbx-tls.yaml";

pub struct TlsConfig {
    pub cf_token: String,
    pub email: String,
}

pub fn tls_config() -> Option<TlsConfig> {
    let cf_token = std::env::var("CLOUDFLARE_DNS_API_TOKEN").ok()?;
    let email = std::env::var("SBX_ACME_EMAIL").ok()?;
    if cf_token.is_empty() || email.is_empty() {
        return None;
    }
    Some(TlsConfig { cf_token, email })
}

pub fn cert_dir() -> PathBuf {
    config_dir().join("proxy-certs")
}

pub fn local_cert_pair() -> Option<(PathBuf, PathBuf)> {
    let dir = cert_dir();
    let cert = dir.join(CERT_FILE);
    let key = dir.join(KEY_FILE);
    if cert.is_file() && key.is_file() {
        Some((cert, key))
    } else {
        None
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum RouteTls {
    None,
    Local,
    Acme,
}

struct TlsState {
    local: bool,
    acme: bool,
}

fn tls_state() -> TlsState {
    TlsState {
        local: local_cert_pair().is_some(),
        acme: tls_config().is_some(),
    }
}

fn is_local_hostname(host: &str) -> bool {
    host == "localhost" || host.ends_with(".localhost")
}

fn route_tls(r: &Route, state: &TlsState) -> RouteTls {
    if r.force_http {
        return RouteTls::None;
    }
    if is_local_hostname(&r.hostname) {
        if state.local {
            RouteTls::Local
        } else {
            RouteTls::None
        }
    } else if state.acme {
        RouteTls::Acme
    } else {
        RouteTls::None
    }
}

fn mkcert_available() -> bool {
    Command::new("mkcert")
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn generate_local_certs() -> Result<(), String> {
    if !mkcert_available() {
        return Err(
            "mkcert not installed. Install with: pacman -S mkcert nss  (or see https://github.com/FiloSottile/mkcert)"
                .into(),
        );
    }
    let dir = cert_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    log("running mkcert -install (may prompt for sudo on first run)");
    let s = Command::new("mkcert")
        .arg("-install")
        .status()
        .map_err(|e| format!("mkcert -install: {e}"))?;
    if !s.success() {
        return Err("mkcert -install failed".into());
    }
    let cert = dir.join(CERT_FILE);
    let key = dir.join(KEY_FILE);
    log(format!("generating local certs at {}", dir.display()));
    let s = Command::new("mkcert")
        .arg("-cert-file")
        .arg(&cert)
        .arg("-key-file")
        .arg(&key)
        .args([
            "*.sbx.localhost",
            "sbx.localhost",
            "*.localhost",
            "localhost",
            "127.0.0.1",
            "::1",
        ])
        .status()
        .map_err(|e| format!("mkcert generate: {e}"))?;
    if !s.success() {
        return Err("mkcert cert generation failed".into());
    }
    log(format!("local cert ready: {}", cert.display()));
    Ok(())
}

pub fn dynamic_dir() -> PathBuf {
    config_dir().join("proxy-dynamic")
}

pub fn acme_dir() -> PathBuf {
    config_dir().join("proxy-acme")
}

fn route_file_path(project_name: &str) -> PathBuf {
    dynamic_dir().join(format!("{}.yaml", sanitize_tag(project_name)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub hostname: String,
    pub path: Option<String>,
    pub port: u16,
    pub force_http: bool,
}

impl crate::project::WorktreeAdjustable for Route {
    fn hostname_mut(&mut self) -> &mut String {
        &mut self.hostname
    }
    fn port_mut(&mut self) -> &mut u16 {
        &mut self.port
    }
}

pub fn read_routes(project_root: &Path) -> Vec<Route> {
    let cfg = crate::config::Config::load_or_default(project_root);
    let body: String = cfg
        .hostname
        .iter()
        .map(|(h, p)| format!("{h} = {p}\n"))
        .collect();
    let mut routes = parse_routes(&body);
    crate::project::apply_worktree_remap(project_root, &mut routes);
    routes
}

pub fn parse_routes(contents: &str) -> Vec<Route> {
    let mut out = Vec::new();
    for line in crate::util::config_lines(contents) {
        let (lhs, port) = match line.split_once('=') {
            Some(v) => v,
            None => continue,
        };
        let mut lhs = lhs.trim();
        let port = port.trim();
        if lhs.is_empty() {
            continue;
        }
        let mut force_http = false;
        if let Some(rest) = lhs.strip_prefix("http://") {
            force_http = true;
            lhs = rest.trim_start();
        } else if let Some(rest) = lhs.strip_prefix("https://") {
            lhs = rest.trim_start();
        }
        if lhs.is_empty() {
            continue;
        }
        let Ok(p) = port.parse::<u16>() else { continue };
        let (host, path) = match lhs.find('/') {
            Some(idx) => {
                let h = lhs[..idx].trim();
                let p = lhs[idx..].trim();
                if h.is_empty() {
                    continue;
                }
                if p == "/" {
                    (h, None)
                } else {
                    (h, Some(p.to_string()))
                }
            }
            None => (lhs, None),
        };
        out.push(Route {
            hostname: host.to_string(),
            path,
            port: p,
            force_http,
        });
    }
    out
}

pub fn hostname_ports(routes: &[Route]) -> std::collections::BTreeSet<u16> {
    routes.iter().map(|r| r.port).collect()
}

pub fn labels_for(project_name: &str, routes: &[Route]) -> Vec<String> {
    if routes.is_empty() {
        return Vec::new();
    }
    let state = tls_state();
    let mut out = Vec::new();
    out.push("--label".into());
    out.push("traefik.enable=true".into());
    out.push("--label".into());
    out.push(format!("traefik.docker.network={NETWORK}"));
    let prefix = sanitize_tag(project_name);
    for (idx, r) in routes.iter().enumerate() {
        let id = format!("{prefix}-{idx}");
        let mode = route_tls(r, &state);
        let entrypoint = match mode {
            RouteTls::None => "web",
            _ => "websecure",
        };
        out.push("--label".into());
        out.push(format!(
            "traefik.http.routers.{id}.rule={}",
            traefik_rule(r)
        ));
        out.push("--label".into());
        out.push(format!(
            "traefik.http.routers.{id}.entrypoints={entrypoint}"
        ));
        match mode {
            RouteTls::Local => {
                out.push("--label".into());
                out.push(format!("traefik.http.routers.{id}.tls=true"));
            }
            RouteTls::Acme => {
                out.push("--label".into());
                out.push(format!("traefik.http.routers.{id}.tls=true"));
                out.push("--label".into());
                out.push(format!(
                    "traefik.http.routers.{id}.tls.certresolver={CERT_RESOLVER}"
                ));
            }
            RouteTls::None => {}
        }
        out.push("--label".into());
        out.push(format!("traefik.http.routers.{id}.service={id}"));
        out.push("--label".into());
        out.push(format!(
            "traefik.http.services.{id}.loadbalancer.server.port={}",
            r.port
        ));
    }
    out
}

fn traefik_rule(r: &Route) -> String {
    match &r.path {
        Some(p) => format!("Host(`{}`) && PathPrefix(`{}`)", r.hostname, p),
        None => format!("Host(`{}`)", r.hostname),
    }
}

pub fn sidecar_running() -> bool {
    crate::docker::container_exists(SIDECAR, false)
}

pub fn sidecar_exists() -> bool {
    crate::docker::container_exists(SIDECAR, true)
}

pub fn attached_count() -> u32 {
    let Ok(out) = Command::new("docker")
        .args([
            "ps",
            "--filter",
            &format!("network={NETWORK}"),
            "--format",
            "{{.Names}}",
        ])
        .output()
    else {
        return 0;
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| {
            let n = l.trim();
            !n.is_empty() && n != SIDECAR
        })
        .count() as u32
}

pub fn network_subnet() -> Option<String> {
    let out = Command::new("docker")
        .args([
            "network",
            "inspect",
            NETWORK,
            "--format",
            "{{(index .IPAM.Config 0).Subnet}}",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

pub fn ensure_network() {
    let exists = Command::new("docker")
        .args(["network", "inspect", NETWORK, "--format", "{{.Name}}"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if exists {
        return;
    }
    let out = Command::new("docker")
        .args(["network", "create", NETWORK])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    match out {
        Ok(o) if o.status.success() => {
            log(format!("created docker network: {NETWORK}"));
        }
        Ok(o) => {
            for line in String::from_utf8_lossy(&o.stderr).lines() {
                log(format!("  docker: {line}"));
            }
            die(format!("failed to create docker network {NETWORK}"));
        }
        Err(e) => die(format!("failed to spawn docker: {e}")),
    }
}

pub fn start_sidecar() {
    ensure_network();
    let dyn_dir = dynamic_dir();
    if let Err(e) = std::fs::create_dir_all(&dyn_dir) {
        die(format!("create {}: {e}", dyn_dir.display()));
    }
    write_dashboard_route();
    if sidecar_running() {
        return;
    }
    if sidecar_exists() {
        crate::docker::force_rm(SIDECAR);
    }
    log(format!("starting proxy sidecar: {SIDECAR}"));
    let dyn_mount = format!("{}:{DYNAMIC_MOUNT}:ro", dyn_dir.display());
    let tls = tls_config();
    let local = local_cert_pair();
    let any_tls = tls.is_some() || local.is_some();
    write_default_cert_yaml(local.is_some());
    let mut cmd = Command::new("docker");
    cmd.args([
        "run",
        "-d",
        "--name",
        SIDECAR,
        "--network",
        NETWORK,
        "-p",
        "127.0.0.1:80:80",
        "-v",
        "/var/run/docker.sock:/var/run/docker.sock:ro",
        "-v",
        &dyn_mount,
    ]);
    if any_tls {
        cmd.args(["-p", "127.0.0.1:443:443"]);
    }
    if let Some((cert, key)) = &local {
        cmd.args(["-v", &format!("{}:/certs/cert.pem:ro", cert.display())]);
        cmd.args(["-v", &format!("{}:/certs/key.pem:ro", key.display())]);
    }
    if let Some(t) = &tls {
        let adir = acme_dir();
        if let Err(e) = std::fs::create_dir_all(&adir) {
            die(format!("create {}: {e}", adir.display()));
        }
        let acme_file = adir.join("acme.json");
        if !acme_file.exists() {
            if let Err(e) = std::fs::write(&acme_file, "") {
                die(format!("create {}: {e}", acme_file.display()));
            }
        }
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&acme_file, std::fs::Permissions::from_mode(0o600))
        {
            log(format!("warn: chmod {}: {e}", acme_file.display()));
        }
        cmd.args(["-v", &format!("{}:/acme/acme.json", acme_file.display())]);
        cmd.args(["-e", &format!("CLOUDFLARE_DNS_API_TOKEN={}", t.cf_token)]);
    }
    cmd.args([
        IMAGE,
        "--api.dashboard=true",
        "--providers.docker=true",
        "--providers.docker.exposedbydefault=false",
        &format!("--providers.docker.network={NETWORK}"),
        &format!("--providers.file.directory={DYNAMIC_MOUNT}"),
        "--providers.file.watch=true",
        "--entrypoints.web.address=:80",
    ]);
    if any_tls {
        cmd.arg("--entrypoints.websecure.address=:443");
    }
    if local.is_some() {
        log("  tls: mkcert local certs mounted at /certs");
    }
    if let Some(t) = &tls {
        cmd.args([
            &format!("--certificatesresolvers.{CERT_RESOLVER}.acme.dnschallenge=true"),
            &format!(
                "--certificatesresolvers.{CERT_RESOLVER}.acme.dnschallenge.provider=cloudflare"
            ),
            &format!(
                "--certificatesresolvers.{CERT_RESOLVER}.acme.dnschallenge.resolvers=1.1.1.1:53,8.8.8.8:53"
            ),
            &format!(
                "--certificatesresolvers.{CERT_RESOLVER}.acme.storage=/acme/acme.json"
            ),
            &format!("--certificatesresolvers.{CERT_RESOLVER}.acme.email={}", t.email),
        ]);
        log(format!(
            "  tls: cloudflare resolver enabled (email={}, storage={})",
            t.email,
            acme_dir().join("acme.json").display()
        ));
    }
    let status = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).output();
    match status {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            for line in String::from_utf8_lossy(&o.stderr).lines() {
                log(format!("  docker: {line}"));
            }
            die("failed to start proxy sidecar");
        }
        Err(e) => die(format!("failed to spawn docker: {e}")),
    }
    for _ in 0..15 {
        if sidecar_listening_on(80) {
            log(format!(
                "proxy sidecar up: {SIDECAR} (http://*.sbx.localhost/)"
            ));
            log(format!("  dashboard: http://{DASHBOARD_HOST}/dashboard/"));
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    log("proxy sidecar didn't bind :80 within 7.5s; continuing anyway");
}

fn write_dashboard_route() {
    let body = format!(
        "http:\n  routers:\n    sbx-dashboard:\n      rule: \"Host(`{DASHBOARD_HOST}`)\"\n      entryPoints:\n        - web\n      service: api@internal\n"
    );
    let path = dynamic_dir().join(DASHBOARD_FILE);
    if let Err(e) = std::fs::write(&path, body) {
        log(format!(
            "warn: write dashboard route {}: {e}",
            path.display()
        ));
    }
}

fn write_default_cert_yaml(enabled: bool) {
    let path = dynamic_dir().join(TLS_DEFAULTS_FILE);
    if !enabled {
        let _ = std::fs::remove_file(&path);
        return;
    }
    let body = "tls:\n  stores:\n    default:\n      defaultCertificate:\n        certFile: /certs/cert.pem\n        keyFile: /certs/key.pem\n  certificates:\n    - certFile: /certs/cert.pem\n      keyFile: /certs/key.pem\n";
    if let Err(e) = std::fs::write(&path, body) {
        log(format!("warn: write tls defaults {}: {e}", path.display()));
    }
}

fn sidecar_listening_on(port: u16) -> bool {
    let addr = format!("127.0.0.1:{port}");
    TcpStream::connect_timeout(&addr.parse().unwrap(), Duration::from_millis(200)).is_ok()
}

pub fn stop_sidecar_if_idle() {
    if !sidecar_running() {
        return;
    }
    let n = attached_count();
    if n > 0 {
        log(format!(
            "proxy sidecar still has {n} attached container(s); leaving {SIDECAR} up"
        ));
        return;
    }
    log(format!("stopping proxy sidecar: {SIDECAR}"));
    crate::docker::force_rm(SIDECAR);
}

pub fn force_stop_sidecar() -> bool {
    crate::docker::stop_if_present(SIDECAR)
}

pub fn attached_containers() -> Vec<String> {
    let Ok(out) = Command::new("docker")
        .args([
            "ps",
            "--filter",
            &format!("network={NETWORK}"),
            "--format",
            "{{.Names}}",
        ])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|n| !n.is_empty() && n != SIDECAR)
        .collect()
}

pub fn route_files() -> Vec<PathBuf> {
    let dir = dynamic_dir();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
        .collect();
    out.sort();
    out
}

pub fn attach_container(container: &str) -> Result<(), String> {
    ensure_network();
    let out = Command::new("docker")
        .args(["network", "connect", NETWORK, container])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    match out {
        Ok(o) if o.status.success() => {
            log(format!("attached {container} to {NETWORK}"));
            Ok(())
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let already = stderr.contains("already exists")
                || stderr.contains("already attached")
                || stderr.contains("endpoint with name");
            if already {
                Ok(())
            } else {
                for line in stderr.lines() {
                    log(format!("  docker: {line}"));
                }
                Err(format!("failed to attach {container} to {NETWORK}"))
            }
        }
        Err(e) => Err(format!("failed to spawn docker: {e}")),
    }
}

pub fn write_file_route(project_name: &str, target_host: &str, routes: &[Route]) -> PathBuf {
    let dir = dynamic_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        die(format!("create {}: {e}", dir.display()));
    }
    let body = render_file_route(project_name, target_host, routes);
    let path = route_file_path(project_name);
    if let Err(e) = std::fs::write(&path, body) {
        die(format!("write {}: {e}", path.display()));
    }
    path
}

pub fn remove_file_route(project_name: &str) {
    let path = route_file_path(project_name);
    let _ = std::fs::remove_file(path);
}

#[cfg(test)]
pub fn render_file_route_for_test(
    project_name: &str,
    target_host: &str,
    routes: &[Route],
) -> String {
    render_file_route(project_name, target_host, routes)
}

fn render_file_route(project_name: &str, target_host: &str, routes: &[Route]) -> String {
    let prefix = sanitize_tag(project_name);
    let state = tls_state();
    let mut routers = String::new();
    let mut services = String::new();
    for (idx, r) in routes.iter().enumerate() {
        let id = format!("{prefix}-{idx}");
        let mode = route_tls(r, &state);
        let entrypoint = match mode {
            RouteTls::None => "web",
            _ => "websecure",
        };
        routers.push_str(&format!(
            "    {id}:\n      rule: \"{rule}\"\n      entryPoints:\n        - {entrypoint}\n      service: {id}\n",
            id = id,
            rule = traefik_rule(r),
            entrypoint = entrypoint,
        ));
        match mode {
            RouteTls::Local => {
                routers.push_str("      tls: {}\n");
            }
            RouteTls::Acme => {
                routers.push_str(&format!(
                    "      tls:\n        certResolver: {CERT_RESOLVER}\n"
                ));
            }
            RouteTls::None => {}
        }
        services.push_str(&format!(
            "    {id}:\n      loadBalancer:\n        servers:\n          - url: \"http://{target_host}:{port}\"\n",
            id = id,
            target_host = target_host,
            port = r.port,
        ));
    }
    format!("http:\n  routers:\n{routers}  services:\n{services}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(host: &str, port: u16) -> Route {
        Route {
            hostname: host.into(),
            path: None,
            port,
            force_http: false,
        }
    }

    fn path_route(host: &str, path: &str, port: u16) -> Route {
        Route {
            hostname: host.into(),
            path: Some(path.into()),
            port,
            force_http: false,
        }
    }

    #[test]
    fn parse_simple() {
        let r = parse_routes("app.sbx.localhost = 3000\napi.app.sbx.localhost=1337\n");
        assert_eq!(
            r,
            vec![
                route("app.sbx.localhost", 3000),
                route("api.app.sbx.localhost", 1337),
            ]
        );
    }

    #[test]
    fn parse_skips_blank_and_comments() {
        let r =
            parse_routes("# a comment\n\n  \nfoo.localhost = 8080 # inline\nbroken\n=3000\nfoo=\n");
        assert_eq!(r, vec![route("foo.localhost", 8080)]);
    }

    #[test]
    fn parse_path_prefix() {
        let r = parse_routes(
            "app.sbx.localhost/api = 1337\n\
             app.sbx.localhost/socket.io = 1337\n\
             app.sbx.localhost = 3000\n",
        );
        assert_eq!(
            r,
            vec![
                path_route("app.sbx.localhost", "/api", 1337),
                path_route("app.sbx.localhost", "/socket.io", 1337),
                route("app.sbx.localhost", 3000),
            ]
        );
    }

    #[test]
    fn parse_http_prefix_marks_force_http() {
        let r = parse_routes("http://api.example.com = 8080\nhttps://app.example.com = 3000\n");
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].hostname, "api.example.com");
        assert!(r[0].force_http);
        assert_eq!(r[1].hostname, "app.example.com");
        assert!(!r[1].force_http);
    }

    #[test]
    fn parse_bare_slash_is_catchall() {
        let r = parse_routes("foo.localhost/ = 3000\n");
        assert_eq!(r, vec![route("foo.localhost", 3000)]);
    }

    #[test]
    fn labels_disambiguate_with_index() {
        let routes = vec![route("a.localhost", 3000), route("b.localhost", 1337)];
        let labels = labels_for("app", &routes);
        let joined = labels.join(" ");
        assert!(joined.contains("traefik.http.routers.app-0.rule=Host(`a.localhost`)"));
        assert!(joined.contains("traefik.http.services.app-0.loadbalancer.server.port=3000"));
        assert!(joined.contains("traefik.http.routers.app-1.rule=Host(`b.localhost`)"));
        assert!(joined.contains("traefik.http.services.app-1.loadbalancer.server.port=1337"));
    }

    #[test]
    fn labels_use_path_prefix_when_set() {
        let routes = vec![path_route("app.sbx.localhost", "/api", 1337)];
        let joined = labels_for("app", &routes).join(" ");
        assert!(joined.contains(
            "traefik.http.routers.app-0.rule=Host(`app.sbx.localhost`) && PathPrefix(`/api`)"
        ));
    }

    #[test]
    fn labels_empty_when_no_routes() {
        assert!(labels_for("app", &[]).is_empty());
    }

    #[test]
    fn file_route_yaml_targets_named_host() {
        let routes = vec![route("a.localhost", 3000), route("b.localhost", 1337)];
        let body = render_file_route("app", "sbx-vpn-host", &routes);
        assert!(body.contains("app-0:"));
        assert!(body.contains("rule: \"Host(`a.localhost`)\""));
        assert!(body.contains("url: \"http://sbx-vpn-host:3000\""));
        assert!(body.contains("app-1:"));
        assert!(body.contains("rule: \"Host(`b.localhost`)\""));
        assert!(body.contains("url: \"http://sbx-vpn-host:1337\""));
        assert!(body.contains("\n  routers:\n"));
        assert!(body.contains("\n  services:\n"));
    }

    #[test]
    fn file_route_yaml_uses_path_prefix() {
        let routes = vec![path_route("app.sbx.localhost", "/api", 1337)];
        let body = render_file_route("app", "sbx-vpn-host", &routes);
        assert!(body.contains("rule: \"Host(`app.sbx.localhost`) && PathPrefix(`/api`)\""));
    }

    #[test]
    fn hostname_ports_dedup() {
        let routes = vec![route("a.localhost", 3000), route("b.localhost", 3000)];
        let ports = hostname_ports(&routes);
        assert_eq!(ports.len(), 1);
        assert!(ports.contains(&3000));
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?}: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn make_repo_with_worktree(label: &str) -> (PathBuf, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let root = std::env::temp_dir().join(format!("sbx-test-proxy-{label}-{pid}-{nanos}"));
        let main = root.join("myapp");
        std::fs::create_dir_all(&main).unwrap();
        run_git(&main, &["init", "-q", "-b", "master"]);
        crate::config::Config::edit(&main, |c| {
            c.hostname.insert("app.sbx.localhost".to_string(), 3000);
        })
        .unwrap();
        run_git(&main, &["add", "."]);
        run_git(&main, &["commit", "-q", "-m", "init"]);
        let wt = root.join("myapp-wt");
        run_git(
            &main,
            &["worktree", "add", "-b", "live", wt.to_str().unwrap()],
        );
        crate::config::Config::edit(&wt, |c| c.port_offset = Some(0)).unwrap();
        (main, wt)
    }

    #[test]
    fn read_routes_prefixes_in_worktree() {
        let (_main, wt) = make_repo_with_worktree("prefix");
        let routes = read_routes(&wt);
        assert_eq!(routes, vec![route("live-app.sbx.localhost", 3000)]);
    }

    #[test]
    fn read_routes_unchanged_in_main_checkout() {
        let (main, _wt) = make_repo_with_worktree("main");
        let routes = read_routes(&main);
        assert_eq!(routes, vec![route("app.sbx.localhost", 3000)]);
    }

    #[test]
    fn read_routes_uses_sbx_name_override() {
        let (_main, wt) = make_repo_with_worktree("name");
        crate::config::Config::edit(&wt, |c| c.name = Some("exp".to_string())).unwrap();
        let routes = read_routes(&wt);
        assert_eq!(routes, vec![route("exp-app.sbx.localhost", 3000)]);
    }

    #[test]
    fn read_routes_applies_explicit_port_offset() {
        let (_main, wt) = make_repo_with_worktree("offset-explicit");
        crate::config::Config::edit(&wt, |c| c.port_offset = Some(7)).unwrap();
        let routes = read_routes(&wt);
        assert_eq!(routes, vec![route("live-app.sbx.localhost", 3007)]);
    }

    #[test]
    fn read_routes_main_checkout_has_zero_offset() {
        let (main, _wt) = make_repo_with_worktree("offset-main");
        // The shared fixture only pins the worktree's offset to 0, so
        // main is left at its natural value — which for branch `master`
        // is also 0.
        let routes = read_routes(&main);
        assert_eq!(routes, vec![route("app.sbx.localhost", 3000)]);
    }

    #[test]
    fn read_routes_falls_back_to_hashed_offset_for_worktree() {
        let (_main, wt) = make_repo_with_worktree("offset-hashed");
        // Clear the pinned-0 offset the fixture set so port_offset falls
        // back to the hash-derived value for branch `live`.
        crate::config::Config::edit(&wt, |c| c.port_offset = None).unwrap();
        let routes = read_routes(&wt);
        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(r.hostname, "live-app.sbx.localhost");
        // Hash-derived offset is in 1..=9, so shifted port is 3001..=3009.
        assert!(
            (3001..=3009).contains(&r.port),
            "expected port in 3001..=3009, got {}",
            r.port
        );
    }
}
