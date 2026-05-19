use std::path::Path;
use std::process::{Command, Stdio};

use crate::project::{project_name, sbx_file};
use crate::util::{die, log};

const IMAGE: &str = "alpine/socat:latest";

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Direction {
    Out,
    In,
    Via,
    ViaHost,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Out => "out",
            Direction::In => "in",
            Direction::Via => "via",
            Direction::ViaHost => "via-host",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "out" => Some(Direction::Out),
            "in" => Some(Direction::In),
            "via" => Some(Direction::Via),
            "via-host" | "viahost" => Some(Direction::ViaHost),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tunnel {
    pub direction: Direction,
    pub left: u16,
    pub right: String,
}

pub fn read_tunnels(project_root: &Path) -> Vec<Tunnel> {
    let f = sbx_file(project_root, "tunnels");
    let Ok(contents) = std::fs::read_to_string(&f) else {
        return Vec::new();
    };
    parse(&contents)
}

pub fn parse(contents: &str) -> Vec<Tunnel> {
    let mut out = Vec::new();
    for line in crate::util::config_lines(contents) {
        let Some((dir_part, rest)) = line.split_once(':') else {
            log(format!("ignoring malformed line in .sbx/tunnels: {line}"));
            continue;
        };
        let Some(direction) = Direction::parse(dir_part) else {
            log(format!(
                "ignoring unknown direction in .sbx/tunnels: {line} (use out/in/via/via-host)"
            ));
            continue;
        };
        let Some((lhs, rhs)) = rest.split_once('=') else {
            log(format!("ignoring malformed line in .sbx/tunnels: {line}"));
            continue;
        };
        let lhs = lhs.trim();
        let rhs = rhs.trim();
        let Ok(left) = lhs.parse::<u16>() else {
            log(format!(
                "ignoring invalid left port in .sbx/tunnels: {line}"
            ));
            continue;
        };
        if rhs.is_empty() {
            log(format!("ignoring empty right side in .sbx/tunnels: {line}"));
            continue;
        }
        match direction {
            Direction::Out | Direction::In => {
                if rhs.parse::<u16>().is_err() {
                    log(format!(
                        "ignoring invalid right port in .sbx/tunnels: {line}"
                    ));
                    continue;
                }
            }
            Direction::Via | Direction::ViaHost => {
                if !rhs.contains(':') {
                    log(format!(
                        "ignoring malformed right side in .sbx/tunnels (need host:port): {line}"
                    ));
                    continue;
                }
                let (_, port) = rhs.rsplit_once(':').unwrap();
                if port.parse::<u16>().is_err() {
                    log(format!(
                        "ignoring invalid right port in .sbx/tunnels: {line}"
                    ));
                    continue;
                }
            }
        }
        out.push(Tunnel {
            direction,
            left,
            right: rhs.to_string(),
        });
    }
    out
}

/// Ports that need `-p 127.0.0.1:host:cont` on the netns owner.
/// - `out`: sandbox-internal port = left, host port = right
/// - `via`: host port = left, in-netns listener port = left (socat listens there)
pub fn publish_args(tunnels: &[Tunnel]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for t in tunnels {
        let (host_port, cont_port) = match t.direction {
            Direction::Out => {
                let right: u16 = t.right.parse().unwrap_or(t.left);
                (right, t.left)
            }
            Direction::Via => (t.left, t.left),
            Direction::In | Direction::ViaHost => continue,
        };
        if !seen.insert((host_port, cont_port)) {
            continue;
        }
        out.push("-p".to_string());
        out.push(format!("127.0.0.1:{host_port}:{cont_port}"));
    }
    out
}

/// Builds the shell command run inside the tunnel sidecar:
/// one socat per in:/via: entry, backgrounded, then `wait`.
/// Returns None if no socat processes are needed.
pub fn socat_script(tunnels: &[Tunnel]) -> Option<String> {
    let mut cmds: Vec<String> = Vec::new();
    for t in tunnels {
        match t.direction {
            Direction::In => {
                let host_port: u16 = match t.right.parse() {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                cmds.push(format!(
                    "socat -d TCP-LISTEN:{l},fork,reuseaddr TCP:host.docker.internal:{r}",
                    l = t.left,
                    r = host_port
                ));
            }
            Direction::Via => {
                cmds.push(format!(
                    "socat -d TCP-LISTEN:{l},fork,reuseaddr TCP:{r}",
                    l = t.left,
                    r = t.right
                ));
            }
            Direction::Out | Direction::ViaHost => {}
        }
    }
    if cmds.is_empty() {
        return None;
    }
    let mut script = String::new();
    for c in &cmds {
        script.push_str(c);
        script.push_str(" &\n");
    }
    script.push_str("wait\n");
    Some(script)
}

pub fn needs_in_netns_forwarder(tunnels: &[Tunnel]) -> bool {
    tunnels
        .iter()
        .any(|t| matches!(t.direction, Direction::In | Direction::Via))
}

pub fn has_via_host_tunnels(tunnels: &[Tunnel]) -> bool {
    tunnels
        .iter()
        .any(|t| matches!(t.direction, Direction::ViaHost))
}

pub fn sidecar_name(pname: &str) -> String {
    format!("sbx-tunnel-{pname}")
}

pub fn sidecar_running(name: &str) -> bool {
    crate::docker::container_exists(name, false)
}

pub fn sidecar_exists(name: &str) -> bool {
    crate::docker::container_exists(name, true)
}

pub fn sidecar_attached_count(sidecar: &str) -> u32 {
    crate::docker::netns_attached_count(sidecar)
}

/// Start the tunnel sidecar.
/// - If `share_netns` is Some, the sidecar joins that netns and only runs socat.
/// - If None, the sidecar owns its own netns and gets the `publish_args` `-p` flags.
pub fn start_sidecar(
    project_root: &Path,
    tunnels: &[Tunnel],
    share_netns: Option<&str>,
    publish_args: &[String],
) -> String {
    let pname = project_name(project_root);
    let sidecar = sidecar_name(&pname);

    let script = match socat_script(tunnels) {
        Some(s) => s,
        None if share_netns.is_none() && !publish_args.is_empty() => {
            // standalone owner with only `out:` entries: nothing for socat to do,
            // but we still need a container to hold the netns + -p flags. sleep forever.
            "sleep infinity\n".to_string()
        }
        None => return String::new(),
    };

    if sidecar_running(&sidecar) {
        log(format!("reusing tunnel sidecar: {sidecar}"));
        return sidecar;
    }
    if sidecar_exists(&sidecar) {
        crate::docker::force_rm(&sidecar);
    }

    log(format!("starting tunnel sidecar: {sidecar}"));
    if let Some(n) = share_netns {
        log(format!("  attaching to netns of: {n}"));
    }
    for t in tunnels {
        log(format!(
            "  {} {} = {}",
            t.direction.as_str(),
            t.left,
            t.right
        ));
    }

    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", &sidecar]);
    cmd.args(["--cap-drop=ALL", "--security-opt=no-new-privileges"]);
    if let Some(owner) = share_netns {
        cmd.args(["--network", &format!("container:{owner}")]);
    } else {
        cmd.arg("--add-host=host.docker.internal:host-gateway");
        for a in publish_args {
            cmd.arg(a);
        }
    }
    cmd.args(["--entrypoint", "sh"]);
    cmd.arg(IMAGE);
    cmd.args(["-c", &script]);

    let out = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).output();
    match out {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            for line in String::from_utf8_lossy(&o.stderr).lines() {
                log(format!("  docker: {line}"));
            }
            die("failed to start tunnel sidecar");
        }
        Err(e) => die(format!("failed to spawn docker: {e}")),
    }
    sidecar
}

pub fn stop_sidecar(sidecar: &str) {
    if sidecar.is_empty() {
        return;
    }
    if !sidecar_exists(sidecar) {
        return;
    }
    log(format!("stopping tunnel sidecar: {sidecar}"));
    crate::docker::stop_if_present(sidecar);
}

pub fn stop_sidecar_if_idle(sidecar: &str) {
    if !sidecar_running(sidecar) {
        return;
    }
    let n = sidecar_attached_count(sidecar);
    if n > 0 {
        log(format!(
            "tunnel sidecar still has {n} attached container(s); leaving {sidecar} up"
        ));
        return;
    }
    stop_sidecar(sidecar);
}

pub fn exposer_name(pname: &str) -> String {
    format!("sbx-tunnel-{pname}-expose")
}

pub fn exposer_exists(name: &str) -> bool {
    crate::docker::container_exists(name, true)
}

pub fn exposer_running(name: &str) -> bool {
    crate::docker::container_exists(name, false)
}

/// Builds the socat script for the bridge-side exposer container.
/// For out: forwards host_port (right) → owner_ip:cont_port (left).
/// For via: forwards host_port (left) → owner_ip:left (the in-netns sidecar listens there).
fn exposer_script(tunnels: &[Tunnel], owner_ip: &str) -> Option<String> {
    let mut cmds: Vec<String> = Vec::new();
    for t in tunnels {
        match t.direction {
            Direction::Out => {
                let cont_port = t.left;
                let host_port: u16 = t.right.parse().unwrap_or(t.left);
                cmds.push(format!(
                    "socat -d TCP-LISTEN:{host_port},fork,reuseaddr TCP:{owner_ip}:{cont_port}"
                ));
            }
            Direction::Via => {
                cmds.push(format!(
                    "socat -d TCP-LISTEN:{l},fork,reuseaddr TCP:{owner_ip}:{l}",
                    l = t.left
                ));
            }
            Direction::In | Direction::ViaHost => {}
        }
    }
    if cmds.is_empty() {
        return None;
    }
    let mut script = String::new();
    for c in &cmds {
        script.push_str(c);
        script.push_str(" &\n");
    }
    script.push_str("wait\n");
    Some(script)
}

/// Start the bridge-side exposer that publishes out:/via: host ports
/// and socats them into the netns owner. Used when the netns owner is a
/// shared sidecar (VPN/TS) we cannot restart to add `-p` flags.
pub fn start_exposer(pname: &str, netns_owner: &str, tunnels: &[Tunnel]) -> Option<String> {
    let publish = publish_args(tunnels);
    if publish.is_empty() {
        return None;
    }
    let owner_ip = match resolve_owner_ip(netns_owner) {
        Some(ip) => ip,
        None => {
            log(format!(
                "tunnel exposer: could not resolve {netns_owner} bridge IP; skipping"
            ));
            return None;
        }
    };
    let script = exposer_script(tunnels, &owner_ip)?;
    let exposer = exposer_name(pname);

    if exposer_running(&exposer) {
        crate::docker::force_rm(&exposer);
    }
    if exposer_exists(&exposer) {
        crate::docker::force_rm(&exposer);
    }

    log(format!("starting tunnel exposer: {exposer}"));
    log(format!("  target: {netns_owner} ({owner_ip})"));

    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", &exposer]);
    cmd.args(["--cap-drop=ALL", "--security-opt=no-new-privileges"]);
    cmd.args(["--add-host", &format!("{netns_owner}:{owner_ip}")]);
    for a in &publish {
        cmd.arg(a);
    }
    cmd.args(["--entrypoint", "sh"]);
    cmd.arg(IMAGE);
    cmd.args(["-c", &script]);

    let out = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).output();
    match out {
        Ok(o) if o.status.success() => Some(exposer),
        Ok(o) => {
            for line in String::from_utf8_lossy(&o.stderr).lines() {
                log(format!("  docker: {line}"));
            }
            log("failed to start tunnel exposer");
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
    log(format!("stopping tunnel exposer: {name}"));
    crate::docker::force_rm(name);
}

pub fn via_host_sidecar_name(pname: &str) -> String {
    format!("sbx-via-host-{pname}")
}

pub fn via_host_sidecar_exists(name: &str) -> bool {
    crate::docker::container_exists(name, true)
}

pub fn via_host_sidecar_running(name: &str) -> bool {
    crate::docker::container_exists(name, false)
}

/// Builds the socat script for the via-host sidecar.
/// Each ViaHost tunnel becomes a socat listening on bridge_ip:left
/// (so only docker-bridge clients can reach it, not LAN-side machines)
/// and forwarding to the remote `host:port` over the host's normal routing.
fn via_host_script(tunnels: &[Tunnel], bridge_ip: &str) -> Option<String> {
    let mut cmds: Vec<String> = Vec::new();
    for t in tunnels {
        if !matches!(t.direction, Direction::ViaHost) {
            continue;
        }
        cmds.push(format!(
            "socat -d TCP-LISTEN:{l},fork,reuseaddr,bind={ip} TCP:{r}",
            l = t.left,
            ip = bridge_ip,
            r = t.right,
        ));
    }
    if cmds.is_empty() {
        return None;
    }
    let mut script = String::new();
    for c in &cmds {
        script.push_str(c);
        script.push_str(" &\n");
    }
    script.push_str("wait\n");
    Some(script)
}

/// Start the via-host sidecar: a socat container on `--network host` that
/// forwards bridge_ip:left -> remote on each ViaHost entry. The sandbox
/// reaches these listeners via `host.docker.internal:left`, which bypasses
/// the VPN sidecar (gluetun's FIREWALL_OUTBOUND_SUBNETS already allows the
/// bridge subnet — see src/vpn.rs).
pub fn start_via_host_sidecar(project_root: &Path, tunnels: &[Tunnel]) -> Option<String> {
    if !has_via_host_tunnels(tunnels) {
        return None;
    }
    let bridge_ip = crate::docker::bridge_gateway();
    let script = via_host_script(tunnels, &bridge_ip)?;

    let pname = project_name(project_root);
    let sidecar = via_host_sidecar_name(&pname);

    if via_host_sidecar_running(&sidecar) {
        crate::docker::force_rm(&sidecar);
    }
    if via_host_sidecar_exists(&sidecar) {
        crate::docker::force_rm(&sidecar);
    }

    log(format!("starting via-host sidecar: {sidecar}"));
    log(format!("  bound to bridge ip: {bridge_ip}"));
    for t in tunnels {
        if matches!(t.direction, Direction::ViaHost) {
            log(format!("  via-host {} = {}", t.left, t.right));
        }
    }

    let mut cmd = Command::new("docker");
    cmd.args(["run", "-d", "--name", &sidecar]);
    cmd.args(["--network", "host"]);
    cmd.args(["--cap-drop=ALL", "--security-opt=no-new-privileges"]);
    cmd.args(["--entrypoint", "sh"]);
    cmd.arg(IMAGE);
    cmd.args(["-c", &script]);

    let out = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).output();
    match out {
        Ok(o) if o.status.success() => Some(sidecar),
        Ok(o) => {
            for line in String::from_utf8_lossy(&o.stderr).lines() {
                log(format!("  docker: {line}"));
            }
            log("failed to start via-host sidecar");
            None
        }
        Err(e) => {
            log(format!("failed to spawn docker: {e}"));
            None
        }
    }
}

pub fn stop_via_host_sidecar(name: &str) {
    if name.is_empty() || !via_host_sidecar_exists(name) {
        return;
    }
    log(format!("stopping via-host sidecar: {name}"));
    crate::docker::force_rm(name);
}

fn container_bridge_ip(name: &str) -> Option<String> {
    // Try the legacy top-level field first.
    if let Ok(out) = Command::new("docker")
        .args([
            "inspect",
            name,
            "--format",
            "{{.NetworkSettings.IPAddress}}",
        ])
        .output()
    {
        let ip = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !ip.is_empty() {
            return Some(ip);
        }
    }
    // Fall back to iterating Networks (handles user-defined networks where
    // the top-level field is empty).
    let out = Command::new("docker")
        .args([
            "inspect",
            name,
            "--format",
            "{{range .NetworkSettings.Networks}}{{.IPAddress}} {{end}}",
        ])
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .find(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn resolve_owner_ip(netns_owner: &str) -> Option<String> {
    for i in 0..10 {
        if let Some(ip) = container_bridge_ip(netns_owner) {
            return Some(ip);
        }
        if i < 9 {
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
    }
    // Diagnostic dump on final failure.
    if let Ok(out) = Command::new("docker")
        .args([
            "inspect",
            netns_owner,
            "--format",
            "{{json .NetworkSettings}}",
        ])
        .output()
    {
        log(format!(
            "  debug: docker inspect {netns_owner} NetworkSettings = {}",
            String::from_utf8_lossy(&out.stdout).trim()
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic() {
        let t = parse(
            "out: 3000 = 3000\n\
             in: 5432 = 5432\n\
             via: 5432 = staging.tail-net.ts.net:5432\n\
             via-host: 27017 = 192.168.1.67:27017\n",
        );
        assert_eq!(
            t,
            vec![
                Tunnel {
                    direction: Direction::Out,
                    left: 3000,
                    right: "3000".into(),
                },
                Tunnel {
                    direction: Direction::In,
                    left: 5432,
                    right: "5432".into(),
                },
                Tunnel {
                    direction: Direction::Via,
                    left: 5432,
                    right: "staging.tail-net.ts.net:5432".into(),
                },
                Tunnel {
                    direction: Direction::ViaHost,
                    left: 27017,
                    right: "192.168.1.67:27017".into(),
                },
            ]
        );
    }

    #[test]
    fn parse_via_host_requires_host_colon_port() {
        // bare port on rhs is rejected
        let t = parse("via-host: 27017 = 27017\n");
        assert!(t.is_empty(), "should reject rhs without ':' for via-host");
        // host without port is rejected
        let t = parse("via-host: 27017 = 192.168.1.67:nope\n");
        assert!(t.is_empty(), "should reject non-numeric port for via-host");
    }

    #[test]
    fn via_host_script_binds_to_bridge_ip() {
        let tunnels = vec![
            Tunnel {
                direction: Direction::ViaHost,
                left: 27017,
                right: "192.168.1.67:27017".into(),
            },
            Tunnel {
                direction: Direction::Out,
                left: 3000,
                right: "3000".into(),
            },
        ];
        let s = via_host_script(&tunnels, "172.17.0.1").expect("script");
        assert!(s.contains(
            "TCP-LISTEN:27017,fork,reuseaddr,bind=172.17.0.1 TCP:192.168.1.67:27017"
        ));
        // out: entries don't belong in this sidecar
        assert!(!s.contains("3000"));
        assert!(s.trim_end().ends_with("wait"));
    }

    #[test]
    fn via_host_script_none_without_via_host_entries() {
        let tunnels = vec![Tunnel {
            direction: Direction::Via,
            left: 5432,
            right: "db.ts.net:5432".into(),
        }];
        assert!(via_host_script(&tunnels, "172.17.0.1").is_none());
    }

    #[test]
    fn has_via_host_tunnels_only_matches_via_host() {
        let none = vec![Tunnel {
            direction: Direction::Via,
            left: 1,
            right: "h:1".into(),
        }];
        assert!(!has_via_host_tunnels(&none));
        let some = vec![Tunnel {
            direction: Direction::ViaHost,
            left: 27017,
            right: "192.168.1.67:27017".into(),
        }];
        assert!(has_via_host_tunnels(&some));
    }

    #[test]
    fn publish_args_skips_via_host() {
        let tunnels = vec![Tunnel {
            direction: Direction::ViaHost,
            left: 27017,
            right: "192.168.1.67:27017".into(),
        }];
        // via-host doesn't need any host-side -p; it binds itself on host net.
        assert!(publish_args(&tunnels).is_empty());
    }

    #[test]
    fn socat_script_skips_via_host() {
        let tunnels = vec![Tunnel {
            direction: Direction::ViaHost,
            left: 27017,
            right: "192.168.1.67:27017".into(),
        }];
        // via-host runs in its own sidecar, not the in-netns one.
        assert!(socat_script(&tunnels).is_none());
    }

    #[test]
    fn parse_skips_blanks_and_comments() {
        let t = parse(
            "# comment\n\
             \n\
             out: 3000 = 3000   # inline\n\
             garbage\n\
             foo: 1 = 2\n\
             in: bad = 5432\n\
             in: 5432 = bad\n\
             via: 5432 = no-port\n",
        );
        assert_eq!(t.len(), 1);
        assert_eq!(t[0].direction, Direction::Out);
    }

    #[test]
    fn publish_args_covers_out_and_via_only() {
        let tunnels = vec![
            Tunnel {
                direction: Direction::Out,
                left: 3000,
                right: "3000".into(),
            },
            Tunnel {
                direction: Direction::Out,
                left: 80,
                right: "8080".into(),
            },
            Tunnel {
                direction: Direction::In,
                left: 5432,
                right: "5432".into(),
            },
            Tunnel {
                direction: Direction::Via,
                left: 6379,
                right: "redis.ts.net:6379".into(),
            },
        ];
        let args = publish_args(&tunnels);
        // 3 -p flags total (out:3000, out:8080, via:6379)
        let p_count = args.iter().filter(|a| a.as_str() == "-p").count();
        assert_eq!(p_count, 3);
        let joined = args.join(" ");
        assert!(joined.contains("127.0.0.1:3000:3000"));
        assert!(joined.contains("127.0.0.1:8080:80"));
        assert!(joined.contains("127.0.0.1:6379:6379"));
    }

    #[test]
    fn socat_script_for_in_and_via() {
        let tunnels = vec![
            Tunnel {
                direction: Direction::Out,
                left: 3000,
                right: "3000".into(),
            },
            Tunnel {
                direction: Direction::In,
                left: 5432,
                right: "5432".into(),
            },
            Tunnel {
                direction: Direction::Via,
                left: 6379,
                right: "redis.ts.net:6379".into(),
            },
        ];
        let s = socat_script(&tunnels).expect("script");
        assert!(s.contains("TCP-LISTEN:5432,fork,reuseaddr TCP:host.docker.internal:5432"));
        assert!(s.contains("TCP-LISTEN:6379,fork,reuseaddr TCP:redis.ts.net:6379"));
        assert!(s.trim_end().ends_with("wait"));
    }

    #[test]
    fn socat_script_none_for_out_only() {
        let tunnels = vec![Tunnel {
            direction: Direction::Out,
            left: 3000,
            right: "3000".into(),
        }];
        assert!(socat_script(&tunnels).is_none());
    }

    #[test]
    fn needs_in_netns_forwarder_matches_in_or_via() {
        let out_only = vec![Tunnel {
            direction: Direction::Out,
            left: 1,
            right: "1".into(),
        }];
        let in_only = vec![Tunnel {
            direction: Direction::In,
            left: 1,
            right: "1".into(),
        }];
        let via_only = vec![Tunnel {
            direction: Direction::Via,
            left: 1,
            right: "h:1".into(),
        }];
        assert!(!needs_in_netns_forwarder(&out_only));
        assert!(needs_in_netns_forwarder(&in_only));
        assert!(needs_in_netns_forwarder(&via_only));
    }
}
