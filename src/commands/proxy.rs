use std::collections::BTreeMap;
use std::process::Command;

use crate::proxy::{self, DASHBOARD_HOST, NETWORK, SIDECAR};
use crate::util::log;

pub enum Action {
    Status,
    Routes,
    Logs { follow: bool },
    Stop,
}

pub fn run(action: Action) {
    match action {
        Action::Status => status(),
        Action::Routes => routes(),
        Action::Logs { follow } => logs(follow),
        Action::Stop => stop(),
    }
}

fn status() {
    let running = proxy::sidecar_running();
    let exists = proxy::sidecar_exists();
    let attached = proxy::attached_containers();
    let route_files = proxy::route_files();

    if running {
        println!("sidecar:   running ({SIDECAR})");
        println!("listen:    http://*.sbx.localhost/  (127.0.0.1:80)");
        println!("dashboard: http://{DASHBOARD_HOST}/dashboard/");
    } else if exists {
        println!("sidecar:   stopped ({SIDECAR})");
    } else {
        println!("sidecar:   not present ({SIDECAR})");
    }
    println!("network:   {NETWORK}");
    println!("attached:  {} container(s)", attached.len());
    for c in &attached {
        println!("  - {c}");
    }
    println!("dynamic:   {} file route(s)", route_files.len());
    for p in &route_files {
        println!("  - {}", p.display());
    }
}

fn routes() {
    let route_files = proxy::route_files();
    let attached = proxy::attached_containers();

    let mut rows: Vec<(String, String, String)> = Vec::new();
    for p in &route_files {
        let body = std::fs::read_to_string(p).unwrap_or_default();
        let stem = p
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        for (rule, target) in parse_yaml_routes(&body) {
            rows.push((stem.clone(), rule, target));
        }
    }
    for c in &attached {
        for (rule, port) in container_routes(c) {
            rows.push((c.clone(), rule, format!("{c}:{port}")));
        }
    }

    if rows.is_empty() {
        log("no routes registered with proxy");
        return;
    }
    let src_w = "SOURCE"
        .len()
        .max(rows.iter().map(|r| r.0.len()).max().unwrap_or(0));
    let rule_w = "RULE"
        .len()
        .max(rows.iter().map(|r| r.1.len()).max().unwrap_or(0));
    println!(
        "{:<sw$}  {:<rw$}  TARGET",
        "SOURCE",
        "RULE",
        sw = src_w,
        rw = rule_w,
    );
    for (src, rule, target) in &rows {
        println!(
            "{:<sw$}  {:<rw$}  {target}",
            src,
            rule,
            sw = src_w,
            rw = rule_w,
        );
    }
}

fn parse_yaml_routes(body: &str) -> Vec<(String, String)> {
    let mut router_rules: BTreeMap<String, String> = BTreeMap::new();
    let mut router_services: BTreeMap<String, String> = BTreeMap::new();
    let mut service_urls: BTreeMap<String, String> = BTreeMap::new();
    let mut section = "";
    let mut current_id: Option<String> = None;
    for line in body.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if indent == 2 && trimmed == "routers:" {
            section = "routers";
            current_id = None;
            continue;
        }
        if indent == 2 && trimmed == "services:" {
            section = "services";
            current_id = None;
            continue;
        }
        if indent == 4 && trimmed.ends_with(':') && !trimmed.contains(' ') {
            current_id = Some(trimmed.trim_end_matches(':').to_string());
            continue;
        }
        let Some(id) = current_id.as_ref() else {
            continue;
        };
        if let Some(rest) = trimmed.strip_prefix("rule:") {
            if section == "routers" {
                router_rules.insert(id.clone(), rest.trim().trim_matches('"').to_string());
            }
        } else if let Some(rest) = trimmed.strip_prefix("service:") {
            if section == "routers" {
                router_services.insert(id.clone(), rest.trim().to_string());
            }
        } else if let Some(rest) = trimmed.strip_prefix("- url:") {
            if section == "services" {
                service_urls.insert(id.clone(), rest.trim().trim_matches('"').to_string());
            }
        }
    }
    let mut out: Vec<(String, String)> = Vec::new();
    for (id, rule) in router_rules {
        let target = router_services
            .get(&id)
            .and_then(|svc| service_urls.get(svc).cloned())
            .or_else(|| service_urls.get(&id).cloned())
            .or_else(|| router_services.get(&id).cloned())
            .unwrap_or_else(|| "?".into());
        out.push((rule, target));
    }
    out
}

fn container_routes(container: &str) -> Vec<(String, String)> {
    let Ok(out) = Command::new("docker")
        .args([
            "inspect",
            container,
            "--format",
            "{{range $k, $v := .Config.Labels}}{{$k}}={{$v}}\n{{end}}",
        ])
        .output()
    else {
        return Vec::new();
    };
    let body = String::from_utf8_lossy(&out.stdout);
    let mut rules: BTreeMap<String, String> = BTreeMap::new();
    let mut ports: BTreeMap<String, String> = BTreeMap::new();
    for line in body.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        if let Some(rest) = k.strip_prefix("traefik.http.routers.") {
            if let Some((id, suffix)) = rest.split_once('.') {
                if suffix == "rule" {
                    rules.insert(id.to_string(), v.to_string());
                }
            }
        } else if let Some(rest) = k.strip_prefix("traefik.http.services.") {
            if let Some((id, suffix)) = rest.split_once('.') {
                if suffix == "loadbalancer.server.port" {
                    ports.insert(id.to_string(), v.to_string());
                }
            }
        }
    }
    let mut out: Vec<(String, String)> = Vec::new();
    for (id, rule) in rules {
        let port = ports.get(&id).cloned().unwrap_or_else(|| "?".into());
        out.push((rule, port));
    }
    out
}

fn logs(follow: bool) {
    if !proxy::sidecar_exists() {
        log(format!("proxy sidecar not present ({SIDECAR})"));
        return;
    }
    let mut cmd = Command::new("docker");
    cmd.arg("logs");
    if follow {
        cmd.arg("-f");
    } else {
        cmd.args(["--tail", "200"]);
    }
    cmd.arg(SIDECAR);
    let _ = cmd.status();
}

fn stop() {
    if proxy::force_stop_sidecar() {
        log(format!("stopped proxy sidecar: {SIDECAR}"));
    } else {
        log(format!("proxy sidecar not present ({SIDECAR})"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::{Route, render_file_route_for_test};

    #[test]
    fn yaml_dashboard_internal_service() {
        let body = "http:\n  routers:\n    sbx-dashboard:\n      rule: \"Host(`traefik.sbx.localhost`)\"\n      entryPoints:\n        - web\n      service: api@internal\n";
        let parsed = parse_yaml_routes(body);
        assert_eq!(
            parsed,
            vec![(
                "Host(`traefik.sbx.localhost`)".into(),
                "api@internal".into(),
            )]
        );
    }

    #[test]
    fn yaml_round_trips() {
        let routes = vec![
            Route {
                hostname: "app.sbx.localhost".into(),
                path: Some("/api".into()),
                port: 1337,
            },
            Route {
                hostname: "app.sbx.localhost".into(),
                path: None,
                port: 3000,
            },
        ];
        let body = render_file_route_for_test("app", "sbx-vpn-host", &routes);
        let parsed = parse_yaml_routes(&body);
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed[0],
            (
                "Host(`app.sbx.localhost`) && PathPrefix(`/api`)".into(),
                "http://sbx-vpn-host:1337".into(),
            )
        );
        assert_eq!(
            parsed[1],
            (
                "Host(`app.sbx.localhost`)".into(),
                "http://sbx-vpn-host:3000".into(),
            )
        );
    }
}
