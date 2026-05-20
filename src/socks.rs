use std::path::Path;
use std::process::{Command, Stdio};

use crate::config::Config;
use crate::project::project_name;
use crate::tunnel;
use crate::util::{die, log};

const IMAGE: &str = "serjs/go-socks5-proxy:latest";
const FORWARD_IMAGE: &str = "alpine/socat:latest";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Socks {
    pub port: u16,
    pub name: Option<String>,
    pub user: Option<String>,
    pub pass: Option<String>,
}

pub fn read_socks(project_root: &Path) -> Vec<Socks> {
    let cfg = Config::load_or_default(project_root);
    parse_config_socks(&cfg.socks)
}

fn parse_config_socks(raws: &[crate::config::Socks]) -> Vec<Socks> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for raw in raws {
        if raw.port == 0 {
            log("ignoring socks entry with port = 0");
            continue;
        }
        if !seen.insert(raw.port) {
            log(format!(
                "ignoring duplicate socks entry on port {}",
                raw.port
            ));
            continue;
        }
        out.push(Socks {
            port: raw.port,
            name: raw.name.clone().filter(|s| !s.is_empty()),
            user: raw.user.clone().filter(|s| !s.is_empty()),
            pass: raw.pass.clone().filter(|s| !s.is_empty()),
        });
    }
    out
}

pub fn publish_args(socks: &[Socks]) -> Vec<String> {
    let mut out = Vec::new();
    for s in socks {
        out.push("-p".to_string());
        out.push(format!("127.0.0.1:{p}:{p}", p = s.port));
    }
    out
}

pub fn sidecar_name(pname: &str, port: u16) -> String {
    format!("sbx-socks-{pname}-{port}")
}

pub fn sidecar_exists(name: &str) -> bool {
    crate::docker::container_exists(name, true)
}

pub fn sidecar_running(name: &str) -> bool {
    crate::docker::container_exists(name, false)
}

pub fn sidecar_attached_count(sidecar: &str) -> u32 {
    crate::docker::netns_attached_count(sidecar)
}

pub fn start_sidecar(
    project_root: &Path,
    socks: &[Socks],
    share_netns: Option<&str>,
) -> Vec<String> {
    if socks.is_empty() {
        return Vec::new();
    }
    let pname = project_name(project_root);
    let mut sidecars = Vec::new();
    let mut netns_target: Option<String> = share_netns.map(String::from);

    for s in socks {
        let name = sidecar_name(&pname, s.port);
        if sidecar_running(&name) {
            log(format!("reusing socks sidecar: {name}"));
            sidecars.push(name.clone());
            if netns_target.is_none() {
                netns_target = Some(name);
            }
            continue;
        }
        if sidecar_exists(&name) {
            crate::docker::force_rm(&name);
        }

        let auth_suffix = if s.user.is_some() && s.pass.is_some() {
            " (auth)"
        } else {
            ""
        };
        let label_suffix = s
            .name
            .as_deref()
            .map(|n| format!("  [{n}]"))
            .unwrap_or_default();
        log(format!("starting socks sidecar: {name}"));
        if let Some(t) = &netns_target {
            if t != &name {
                log(format!("  attaching to netns of: {t}"));
            }
        }
        log(format!(
            "  socks5://127.0.0.1:{}{auth_suffix}{label_suffix}",
            s.port
        ));

        let mut cmd = Command::new("docker");
        cmd.args(["run", "-d", "--name", &name]);
        cmd.args(["--cap-drop=ALL", "--security-opt=no-new-privileges"]);
        if let Some(owner) = &netns_target {
            cmd.args(["--network", &format!("container:{owner}")]);
        }
        cmd.args(["-e", &format!("PROXY_PORT={}", s.port)]);
        if let (Some(u), Some(p)) = (&s.user, &s.pass) {
            cmd.args(["-e", "REQUIRE_AUTH=true"]);
            cmd.args(["-e", &format!("PROXY_USER={u}")]);
            cmd.args(["-e", &format!("PROXY_PASSWORD={p}")]);
        } else {
            cmd.args(["-e", "REQUIRE_AUTH=false"]);
        }
        cmd.arg(IMAGE);

        let out = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).output();
        match out {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                for line in String::from_utf8_lossy(&o.stderr).lines() {
                    log(format!("  docker: {line}"));
                }
                die(format!("failed to start socks sidecar: {name}"));
            }
            Err(e) => die(format!("failed to spawn docker: {e}")),
        }

        sidecars.push(name.clone());
        if netns_target.is_none() {
            netns_target = Some(name);
        }
    }

    if let Some(s) = socks
        .iter()
        .find(|s| matches!(s.name.as_deref(), Some("mongo")))
    {
        log(format!(
            "  hint: mongodb+srv://USER:PW@HOST/?proxyHost=127.0.0.1&proxyPort={}",
            s.port
        ));
    }

    sidecars
}

pub fn stop_sidecars_if_idle(sidecars: &[String]) {
    for name in sidecars.iter().rev() {
        if !sidecar_running(name) {
            continue;
        }
        let n = sidecar_attached_count(name);
        if n > 0 {
            log(format!(
                "socks sidecar still has {n} attached container(s); leaving {name} up"
            ));
            continue;
        }
        log(format!("stopping socks sidecar: {name}"));
        crate::docker::stop_if_present(name);
    }
}

pub fn exposer_name(pname: &str) -> String {
    format!("sbx-socks-{pname}-expose")
}

pub fn exposer_exists(name: &str) -> bool {
    crate::docker::container_exists(name, true)
}

pub fn exposer_running(name: &str) -> bool {
    crate::docker::container_exists(name, false)
}

fn exposer_script(socks: &[Socks], owner_ip: &str) -> Option<String> {
    if socks.is_empty() {
        return None;
    }
    let mut s = String::new();
    for sock in socks {
        s.push_str(&format!(
            "socat -d TCP-LISTEN:{p},fork,reuseaddr TCP:{owner_ip}:{p} &\n",
            p = sock.port,
        ));
    }
    s.push_str("wait\n");
    Some(s)
}

pub fn start_exposer(pname: &str, netns_owner: &str, socks: &[Socks]) -> Option<String> {
    if socks.is_empty() {
        return None;
    }
    let owner_ip = match tunnel::resolve_owner_ip(netns_owner) {
        Some(ip) => ip,
        None => {
            log(format!(
                "socks exposer: could not resolve {netns_owner} bridge IP; skipping"
            ));
            return None;
        }
    };
    let script = exposer_script(socks, &owner_ip)?;
    let exposer = exposer_name(pname);

    if exposer_running(&exposer) {
        crate::docker::force_rm(&exposer);
    }
    if exposer_exists(&exposer) {
        crate::docker::force_rm(&exposer);
    }

    log(format!("starting socks exposer: {exposer}"));
    log(format!("  target: {netns_owner} ({owner_ip})"));

    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", &exposer]);
    cmd.args(["--cap-drop=ALL", "--security-opt=no-new-privileges"]);
    cmd.args(["--add-host", &format!("{netns_owner}:{owner_ip}")]);
    for a in publish_args(socks) {
        cmd.arg(a);
    }
    cmd.args(["--entrypoint", "sh"]);
    cmd.arg(FORWARD_IMAGE);
    cmd.args(["-c", &script]);

    let out = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).output();
    match out {
        Ok(o) if o.status.success() => Some(exposer),
        Ok(o) => {
            for line in String::from_utf8_lossy(&o.stderr).lines() {
                log(format!("  docker: {line}"));
            }
            log("failed to start socks exposer");
            None
        }
        Err(e) => {
            log(format!("failed to spawn docker: {e}"));
            None
        }
    }
}

pub fn stop_exposer(name: &str) {
    if name.is_empty() || !exposer_exists(name) {
        return;
    }
    log(format!("stopping socks exposer: {name}"));
    crate::docker::force_rm(name);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_toml(s: &str) -> Vec<Socks> {
        let cfg: crate::config::Config = toml::from_str(s).expect("parse");
        parse_config_socks(&cfg.socks)
    }

    #[test]
    fn parse_basic() {
        let s = parse_toml(
            "[[socks]]\nport = 1080\nname = \"mongo\"\n\
             [[socks]]\nport = 1081\nuser = \"alice\"\npass = \"secret\"\n",
        );
        assert_eq!(
            s,
            vec![
                Socks {
                    port: 1080,
                    name: Some("mongo".into()),
                    user: None,
                    pass: None,
                },
                Socks {
                    port: 1081,
                    name: None,
                    user: Some("alice".into()),
                    pass: Some("secret".into()),
                },
            ]
        );
    }

    #[test]
    fn drops_duplicates_and_zero_port() {
        let s = parse_toml(
            "[[socks]]\nport = 1080\n\
             [[socks]]\nport = 1080\n\
             [[socks]]\nport = 0\n",
        );
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].port, 1080);
    }

    #[test]
    fn publish_args_emits_loopback() {
        let s = vec![
            Socks {
                port: 1080,
                name: None,
                user: None,
                pass: None,
            },
            Socks {
                port: 1081,
                name: None,
                user: None,
                pass: None,
            },
        ];
        assert_eq!(
            publish_args(&s),
            vec![
                "-p".to_string(),
                "127.0.0.1:1080:1080".to_string(),
                "-p".to_string(),
                "127.0.0.1:1081:1081".to_string(),
            ]
        );
    }
}
