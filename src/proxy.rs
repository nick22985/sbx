use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::project::sbx_file;
use crate::util::{config_dir, die, log, sanitize_tag};

pub const NETWORK: &str = "sbx-proxy-net";
pub const SIDECAR: &str = "sbx-proxy";
pub const DASHBOARD_HOST: &str = "traefik.sbx.localhost";
const IMAGE: &str = "traefik:v3";
const DYNAMIC_MOUNT: &str = "/etc/traefik/dynamic";
const DASHBOARD_FILE: &str = "_sbx-dashboard.yaml";

pub fn dynamic_dir() -> PathBuf {
    config_dir().join("proxy-dynamic")
}

fn route_file_path(project_name: &str) -> PathBuf {
    dynamic_dir().join(format!("{}.yaml", sanitize_tag(project_name)))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    pub hostname: String,
    pub path: Option<String>,
    pub port: u16,
}

pub fn read_routes(project_root: &Path) -> Vec<Route> {
    let f = sbx_file(project_root, "hostname");
    let Ok(contents) = std::fs::read_to_string(&f) else {
        return Vec::new();
    };
    parse_routes(&contents)
}

pub fn parse_routes(contents: &str) -> Vec<Route> {
    let mut out = Vec::new();
    for raw in contents.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let (lhs, port) = match line.split_once('=') {
            Some(v) => v,
            None => continue,
        };
        let lhs = lhs.trim();
        let port = port.trim();
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
    let mut out = Vec::new();
    out.push("--label".into());
    out.push("traefik.enable=true".into());
    out.push("--label".into());
    out.push(format!("traefik.docker.network={NETWORK}"));
    let prefix = sanitize_tag(project_name);
    for (idx, r) in routes.iter().enumerate() {
        let id = format!("{prefix}-{idx}");
        out.push("--label".into());
        out.push(format!(
            "traefik.http.routers.{id}.rule={}",
            traefik_rule(r)
        ));
        out.push("--label".into());
        out.push(format!("traefik.http.routers.{id}.entrypoints=web"));
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
    container_exists(false)
}

pub fn sidecar_exists() -> bool {
    container_exists(true)
}

fn container_exists(include_stopped: bool) -> bool {
    let mut cmd = Command::new("docker");
    cmd.arg("ps");
    if include_stopped {
        cmd.arg("-a");
    }
    cmd.args([
        "--filter",
        &format!("name=^{SIDECAR}$"),
        "--format",
        "{{.Names}}",
    ]);
    let Ok(out) = cmd.output() else {
        return false;
    };
    !String::from_utf8_lossy(&out.stdout).trim().is_empty()
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

fn ensure_network() {
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
        force_rm(SIDECAR);
    }
    log(format!("starting proxy sidecar: {SIDECAR}"));
    let dyn_mount = format!("{}:{DYNAMIC_MOUNT}:ro", dyn_dir.display());
    let status = Command::new("docker")
        .args([
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
            "-e",
            IMAGE,
            "--api.dashboard=true",
            "--providers.docker=true",
            "--providers.docker.exposedbydefault=false",
            &format!("--providers.docker.network={NETWORK}"),
            &format!("--providers.file.directory={DYNAMIC_MOUNT}"),
            "--providers.file.watch=true",
            "--entrypoints.web.address=:80",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
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
        if container_listening_on_port(SIDECAR, 80) {
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

fn container_listening_on_port(container: &str, port: u16) -> bool {
    let out = Command::new("docker")
        .args([
            "exec",
            container,
            "sh",
            "-c",
            &format!("(echo > /dev/tcp/127.0.0.1/{port}) 2>/dev/null"),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    matches!(out, Ok(s) if s.success())
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
    force_rm(SIDECAR);
}

pub fn force_stop_sidecar() -> bool {
    if !sidecar_exists() {
        return false;
    }
    force_rm(SIDECAR);
    true
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

fn force_rm(name: &str) {
    let _ = Command::new("docker")
        .args(["rm", "-f", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn attach_container(container: &str) {
    ensure_network();
    let out = Command::new("docker")
        .args(["network", "connect", NETWORK, container])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    match out {
        Ok(o) if o.status.success() => {
            log(format!("attached {container} to {NETWORK}"));
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let already = stderr.contains("already exists")
                || stderr.contains("already attached")
                || stderr.contains("endpoint with name");
            if !already {
                for line in stderr.lines() {
                    log(format!("  docker: {line}"));
                }
                die(format!("failed to attach {container} to {NETWORK}"));
            }
        }
        Err(e) => die(format!("failed to spawn docker: {e}")),
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
    let mut routers = String::new();
    let mut services = String::new();
    for (idx, r) in routes.iter().enumerate() {
        let id = format!("{prefix}-{idx}");
        routers.push_str(&format!(
            "    {id}:\n      rule: \"{rule}\"\n      entryPoints:\n        - web\n      service: {id}\n",
            id = id,
            rule = traefik_rule(r),
        ));
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
        }
    }

    fn path_route(host: &str, path: &str, port: u16) -> Route {
        Route {
            hostname: host.into(),
            path: Some(path.into()),
            port,
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
}
