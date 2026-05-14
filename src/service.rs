use std::path::Path;
use std::process::{Command, Stdio};

use crate::project::{project_name, sbx_file};
use crate::util::{die, log};

pub const BUILTIN_SERVICES: &[&str] = &["redis", "postgres", "mongo", "mysql", "mailpit"];

pub struct ServiceSpec {
    pub image: String,
    pub envs: Vec<String>,
}

pub fn service_template(spec: &str) -> Option<ServiceSpec> {
    match spec {
        "redis" => Some(ServiceSpec {
            image: "redis:alpine".into(),
            envs: vec![],
        }),
        "postgres" | "postgresql" => Some(ServiceSpec {
            image: "postgres:16-alpine".into(),
            envs: vec![
                "POSTGRES_PASSWORD=postgres".into(),
                "POSTGRES_USER=postgres".into(),
                "POSTGRES_DB=postgres".into(),
            ],
        }),
        "mongo" | "mongodb" => Some(ServiceSpec {
            image: "mongo:7".into(),
            envs: vec![],
        }),
        "mysql" => Some(ServiceSpec {
            image: "mysql:8".into(),
            envs: vec!["MYSQL_ALLOW_EMPTY_PASSWORD=yes".into()],
        }),
        "mailpit" => Some(ServiceSpec {
            image: "axllent/mailpit".into(),
            envs: vec![],
        }),
        other => {
            if other.contains('/') || other.contains(':') {
                Some(ServiceSpec {
                    image: other.into(),
                    envs: vec![],
                })
            } else {
                None
            }
        }
    }
}

pub fn project_services(project_root: &Path) -> Vec<String> {
    let file = sbx_file(project_root, "services");
    let Ok(content) = std::fs::read_to_string(&file) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for raw in content.lines() {
        let no_comment = raw.split('#').next().unwrap_or("");
        let trimmed = no_comment.trim();
        if !trimmed.is_empty() {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn service_short_name(spec: &str) -> String {
    let mut s = spec.rsplit('/').next().unwrap_or(spec).to_string();
    if let Some(i) = s.find(':') {
        s.truncate(i);
    }
    if let Some(i) = s.find('@') {
        s.truncate(i);
    }
    let mut out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    while out.contains("--") {
        out = out.replace("--", "-");
    }
    out.trim_matches('-').to_string()
}

pub fn service_container_name(pname: &str, short: &str) -> String {
    format!("sbx-svc-{pname}-{short}")
}

pub fn start_service(
    spec: &str,
    project_root: &Path,
    netns_owner: Option<&str>,
    publish_args: &[String],
) -> String {
    let pname = project_name(project_root);
    let short = service_short_name(spec);
    let cname = service_container_name(&pname, &short);
    if is_running(&cname) {
        log(format!("service already up: {cname}"));
        return cname;
    }
    let _ = Command::new("docker")
        .args(["rm", "-f", &cname])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let Some(tpl) = service_template(spec) else {
        die(format!(
            "unknown service: '{spec}' (built-ins: {}; or pass an image like 'ghcr.io/foo/bar:tag')",
            BUILTIN_SERVICES.join(" ")
        ));
    };
    log(format!("starting service: {cname} (image: {})", tpl.image));
    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", &cname]);
    cmd.args(["--cap-drop=ALL", "--security-opt=no-new-privileges"]);
    for e in &tpl.envs {
        cmd.args(["-e", e]);
    }
    if let Some(owner) = netns_owner {
        cmd.args(["--network", &format!("container:{owner}")]);
    }
    for a in publish_args {
        cmd.arg(a);
    }
    cmd.arg(&tpl.image);
    let out = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).status();
    match out {
        Ok(s) if s.success() => cname,
        _ => die(format!("failed to start service: {cname}")),
    }
}

pub fn stop_service(cname: &str) {
    if cname.is_empty() {
        return;
    }
    let _ = Command::new("docker")
        .args(["rm", "-f", cname])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn stop_all_for_project(pname: &str) {
    let Ok(out) = Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("name=^sbx-svc-{pname}-"),
            "--format",
            "{{.Names}}",
        ])
        .output()
    else {
        return;
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let c = line.trim();
        if c.is_empty() {
            continue;
        }
        log(format!("stopping service: {c}"));
        let _ = Command::new("docker")
            .args(["rm", "-f", c])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn is_running(name: &str) -> bool {
    let Ok(out) = Command::new("docker")
        .args([
            "ps",
            "--filter",
            &format!("name=^{name}$"),
            "--format",
            "{{.Names}}",
        ])
        .output()
    else {
        return false;
    };
    !String::from_utf8_lossy(&out.stdout).trim().is_empty()
}
