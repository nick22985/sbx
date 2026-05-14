use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::docker::{bridge_subnet, PortSpec};
use crate::network::ProjectNetwork;
use crate::project::project_name;
use crate::util::{die, expand_tilde, home_dir, log};

pub fn resolve_ovpn(spec: &str) -> Option<PathBuf> {
    if spec.is_empty() {
        return None;
    }
    if spec.starts_with('/') || spec.starts_with("~/") {
        return Some(expand_tilde(spec));
    }
    let base = std::env::var("SBX_VPN_DIR").ok()?;
    let base = expand_tilde(&base);
    Some(base.join(format!("{spec}.ovpn")))
}

pub fn project_vpn_spec(project_root: &Path) -> Option<String> {
    ProjectNetwork::read(project_root).vpn
}

pub fn sidecar_name(pname: &str) -> String {
    format!("sbx-vpn-{pname}")
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
    let Ok(out) = cmd.output() else {
        return false;
    };
    !String::from_utf8_lossy(&out.stdout).trim().is_empty()
}

pub fn sidecar_attached_count(sidecar: &str) -> u32 {
    let Ok(out) = Command::new("docker")
        .args([
            "ps",
            "--filter",
            &format!("network=container:{sidecar}"),
            "--format",
            "{{.Names}}",
        ])
        .output()
    else {
        return 0;
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count() as u32
}

pub fn start_sidecar(spec: &str, project_root: &Path) -> String {
    let pname = project_name(project_root);
    let sidecar = sidecar_name(&pname);
    if sidecar_running(&sidecar) {
        let attached = sidecar_attached_count(&sidecar);
        if attached > 0 {
            log(format!(
                "reusing vpn sidecar: {sidecar} (serving {attached} container(s))"
            ));
            return sidecar;
        }
        log(format!("vpn sidecar {sidecar} is up but idle; recycling"));
        force_rm(&sidecar);
    }
    let ovpn = resolve_ovpn(spec).unwrap_or_else(|| {
        die(format!(
            "cannot resolve '{spec}' — set SBX_VPN_DIR or use a full path"
        ))
    });
    if !ovpn.is_file() {
        die(format!("no VPN config: {}", ovpn.display()));
    }
    let auth = ovpn.with_extension({
        let mut s = ovpn
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !s.is_empty() {
            s.push('.');
        }
        s.push_str("auth");
        s
    });
    let auth = if ovpn.extension().is_some() {
        let mut s = ovpn.as_os_str().to_owned();
        s.push(".auth");
        PathBuf::from(s)
    } else {
        auth
    };
    let mut auth_envs: Vec<String> = Vec::new();
    if auth.is_file()
        && let Ok(text) = std::fs::read_to_string(&auth)
    {
        let mut lines = text.lines();
        let user = lines.next().unwrap_or("").to_string();
        let pass: String = lines.collect::<Vec<_>>().join("\n");
        if !user.is_empty() {
            auth_envs.push("-e".into());
            auth_envs.push(format!("OPENVPN_USER={user}"));
        }
        if !pass.is_empty() {
            auth_envs.push("-e".into());
            auth_envs.push(format!("OPENVPN_PASSWORD={pass}"));
        }
    }
    let subnet = bridge_subnet();

    log(format!("starting vpn sidecar: {sidecar}"));
    log(format!("  config: {}", ovpn.display()));
    log(format!("  allowing host bridge: {subnet}"));

    if sidecar_exists(&sidecar) {
        force_rm(&sidecar);
    }

    let ports = PortSpec::from_project(project_root).to_docker_args();
    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", &sidecar]);
    cmd.args([
        "--cap-add=NET_ADMIN",
        "--device=/dev/net/tun",
        "--add-host=host.docker.internal:host-gateway",
        "-e",
        "VPN_SERVICE_PROVIDER=custom",
        "-e",
        "VPN_TYPE=openvpn",
        "-e",
        "OPENVPN_CUSTOM_CONFIG=/gluetun/custom.conf",
    ]);
    cmd.args(["-e", &format!("FIREWALL_OUTBOUND_SUBNETS={subnet}")]);
    for a in &auth_envs {
        cmd.arg(a);
    }
    cmd.arg("-v")
        .arg(format!("{}:/gluetun/custom.conf:ro", ovpn.display()));
    for a in ports {
        cmd.arg(a);
    }
    cmd.arg("qmcgaw/gluetun");
    let status = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).status();
    if status.map(|s| !s.success()).unwrap_or(true) {
        die("failed to start vpn sidecar");
    }

    for _ in 0..30 {
        if !sidecar_running(&sidecar) {
            log("vpn sidecar exited unexpectedly; logs:");
            print_logs(&sidecar);
            log(format!(
                "(container kept for inspection: docker logs {sidecar}; clean with: docker rm {sidecar})"
            ));
            die("vpn sidecar failed to start");
        }
        let ok = Command::new("docker")
            .args([
                "exec",
                &sidecar,
                "wget",
                "-qO-",
                "-T",
                "2",
                "https://api.ipify.org",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            log(format!("vpn sidecar up: {sidecar}"));
            return sidecar;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    log("vpn sidecar didn't reach the internet in 30s; logs:");
    print_logs(&sidecar);
    force_rm(&sidecar);
    die("vpn sidecar didn't come up in 30s");
}

pub fn stop_sidecar(sidecar: &str) {
    if !sidecar_exists(sidecar) {
        return;
    }
    log(format!("stopping vpn sidecar: {sidecar}"));
    force_rm(sidecar);
}

pub fn stop_sidecar_if_idle(sidecar: &str) {
    if !sidecar_running(sidecar) {
        return;
    }
    let n = sidecar_attached_count(sidecar);
    if n > 0 {
        log(format!(
            "vpn sidecar still has {n} attached container(s); leaving {sidecar} up"
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

pub fn inline_ovpn(target: &Path) -> Result<InlineStats, String> {
    let content =
        std::fs::read_to_string(target).map_err(|e| format!("read {}: {e}", target.display()))?;
    let mut stats = InlineStats::default();
    let mut out = String::new();
    let _home = home_dir();
    for raw in content.lines() {
        let unquoted = strip_nmcli_quotes(raw);
        if unquoted != raw {
            stats.unquoted += 1;
        }
        let tokens: Vec<&str> = unquoted.split_whitespace().collect();
        let dir = tokens.first().copied().unwrap_or("");
        match dir {
            "user" | "group" | "up" | "down" | "script-security" => {
                log(format!("dropping host-side directive: {unquoted}"));
                stats.dropped += 1;
                continue;
            }
            "auth-user-pass" if tokens.len() >= 2 => {
                out.push_str("auth-user-pass\n");
                log(format!("rewrote: {unquoted}  ->  auth-user-pass"));
                stats.dropped += 1;
                continue;
            }
            "ca" | "cert" | "key" | "tls-auth" | "tls-crypt" | "tls-crypt-v2" => {
                if let Some(path_raw) = tokens.get(1) {
                    let path = strip_quotes(path_raw);
                    let kd = tokens.get(2).map(|s| strip_quotes(s));
                    let p = PathBuf::from(&path);
                    if p.is_file() {
                        let body = std::fs::read_to_string(&p)
                            .map_err(|e| format!("read {}: {e}", p.display()))?;
                        out.push('<');
                        out.push_str(dir);
                        out.push_str(">\n");
                        out.push_str(&body);
                        if !body.ends_with('\n') {
                            out.push('\n');
                        }
                        out.push_str("</");
                        out.push_str(dir);
                        out.push_str(">\n");
                        if dir == "tls-auth"
                            && let Some(k) = kd
                        {
                            out.push_str(&format!("key-direction {k}\n"));
                        }
                        stats.replaced += 1;
                        continue;
                    } else {
                        log(format!(
                            "missing {dir} file: {path} (leaving directive as-is)"
                        ));
                        stats.missing += 1;
                    }
                }
            }
            _ => {}
        }
        out.push_str(&unquoted);
        out.push('\n');
    }

    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let tmp = parent.join(format!(".sbx-vpn-inline.{}.tmp", std::process::id()));
    std::fs::write(&tmp, &out).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(target)
        .ok()
        .map(|m| m.permissions().mode())
        .unwrap_or(0o600);
    let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
    std::fs::rename(&tmp, target).map_err(|e| format!("rename: {e}"))?;
    Ok(stats)
}

#[derive(Default)]
pub struct InlineStats {
    pub replaced: u32,
    pub missing: u32,
    pub unquoted: u32,
    pub dropped: u32,
}

fn strip_nmcli_quotes(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            let mut buf = String::new();
            let mut closed = false;
            for c2 in chars.by_ref() {
                if c2 == '\'' {
                    closed = true;
                    break;
                }
                if c2.is_whitespace() {
                    out.push('\'');
                    out.push_str(&buf);
                    out.push(c2);
                    buf.clear();
                    closed = true;
                    break;
                }
                buf.push(c2);
            }
            if closed && !buf.is_empty() {
                out.push_str(&buf);
            } else if !closed {
                out.push('\'');
                out.push_str(&buf);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn strip_quotes(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return s[1..s.len() - 1].to_string();
        }
    }
    s.to_string()
}
