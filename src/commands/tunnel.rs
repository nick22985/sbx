use std::collections::HashSet;
use std::path::Path;
use std::process::Command;

use crate::config::{Config, Tunnel as ConfigTunnel, TunnelRight};
use crate::docker;
use crate::project::{project_flavor, project_name};
use crate::tunnel::{self, Direction};
use crate::util::{die, log};

pub enum Action<'a> {
    List,
    Add {
        direction: &'a str,
        left: &'a str,
        right: &'a str,
    },
    Remove {
        direction: &'a str,
        left: &'a str,
    },
}

pub enum TopAction {
    Status,
    Logs { follow: bool },
    Stop,
}

pub fn run_top(cwd: &Path, action: TopAction) {
    let (flavor, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));
    match action {
        TopAction::Status => status(&root, &flavor),
        TopAction::Logs { follow } => logs(&root, follow),
        TopAction::Stop => stop(&root),
    }
}

fn status(project_root: &Path, flavor: &str) {
    let pname = project_name(project_root);
    let sidecar = tunnel::sidecar_name(&pname);
    let exposer = tunnel::exposer_name(&pname);
    let via_host = tunnel::via_host_sidecar_name(&pname);
    let tunnels = tunnel::read_tunnels(project_root);

    let main = docker::find_running_container(flavor, &pname);
    let netns_owner = main
        .as_ref()
        .and_then(|c| container_netns_owner(c).or_else(|| Some(c.clone())));

    println!("project:  {pname}");
    match (&main, &netns_owner) {
        (Some(m), Some(o)) if o == m => println!("session:  running ({m}; owns netns)"),
        (Some(m), Some(o)) => println!("session:  running ({m}; netns: {o})"),
        _ => println!("session:  not running"),
    }
    let needs_sidecar = tunnel::needs_in_netns_forwarder(&tunnels);
    if needs_sidecar || tunnel::sidecar_exists(&sidecar) {
        println!("sidecar:  {}", crate::docker::state_line(&sidecar));
    }
    if tunnel::exposer_exists(&exposer) {
        println!("exposer:  {}", crate::docker::state_line(&exposer));
    }
    let needs_via_host = tunnel::has_via_host_tunnels(&tunnels);
    if needs_via_host || tunnel::via_host_sidecar_exists(&via_host) {
        println!("via-host: {}", crate::docker::state_line(&via_host));
    }

    if tunnels.is_empty() {
        println!("tunnels:  (none configured)");
        return;
    }

    // Published host ports may be on the exposer (VPN/TS case) OR on the netns owner (no shared
    // sidecar case). Union of both so the status is correct either way.
    let mut published: HashSet<u16> = netns_owner
        .as_ref()
        .map(|c| published_host_ports(c))
        .unwrap_or_default();
    if tunnel::exposer_running(&exposer) {
        published.extend(published_host_ports(&exposer));
    }
    let sidecar_listening: HashSet<u16> = if tunnel::sidecar_running(&sidecar) {
        listening_ports(&sidecar)
    } else {
        HashSet::new()
    };
    let via_host_listening: HashSet<u16> = if tunnel::via_host_sidecar_running(&via_host) {
        via_host_listening_ports(&via_host)
    } else {
        HashSet::new()
    };

    println!("tunnels:");
    let dir_w = tunnels
        .iter()
        .map(|t| t.direction.as_str().len())
        .max()
        .unwrap_or(3);
    let left_w = tunnels
        .iter()
        .map(|t| t.left.to_string().len())
        .max()
        .unwrap_or(5);
    let arrows: Vec<String> = tunnels
        .iter()
        .map(|t| match t.direction {
            Direction::Out => format!("sbx:{} -> host 127.0.0.1:{}", t.left, t.right),
            Direction::In => format!("host:{} -> sbx :{}", t.right, t.left),
            Direction::Via => format!("host 127.0.0.1:{} -> {}", t.left, t.right),
            Direction::ViaHost => {
                format!("sbx -> host.docker.internal:{} -> {}", t.left, t.right)
            }
        })
        .collect();
    let arrow_w = arrows.iter().map(|a| a.len()).max().unwrap_or(0);
    for (t, arrow) in tunnels.iter().zip(arrows.iter()) {
        let state = tunnel_state(
            t,
            main.is_some(),
            &published,
            &sidecar_listening,
            &via_host_listening,
        );
        println!(
            "  {:<dw$}  {:<lw$}  {:<aw$}  [{state}]",
            t.direction.as_str(),
            t.left,
            arrow,
            dw = dir_w,
            lw = left_w,
            aw = arrow_w,
        );
    }
}

fn tunnel_state(
    t: &tunnel::Tunnel,
    session_running: bool,
    published: &HashSet<u16>,
    sidecar_listening: &HashSet<u16>,
    via_host_listening: &HashSet<u16>,
) -> &'static str {
    if !session_running {
        return "down";
    }
    match t.direction {
        Direction::Out => {
            let hp = t.right.parse::<u16>().unwrap_or(t.left);
            if published.contains(&hp) {
                "active"
            } else {
                "down"
            }
        }
        Direction::In => {
            if sidecar_listening.contains(&t.left) {
                "active"
            } else {
                "down"
            }
        }
        Direction::Via => {
            let p = published.contains(&t.left);
            let l = sidecar_listening.contains(&t.left);
            match (p, l) {
                (true, true) => "active",
                (false, false) => "down",
                _ => "partial",
            }
        }
        Direction::ViaHost => {
            if via_host_listening.contains(&t.left) {
                "active"
            } else {
                "down"
            }
        }
    }
}

fn container_netns_owner(container: &str) -> Option<String> {
    let out = Command::new("docker")
        .args([
            "inspect",
            container,
            "--format",
            "{{.HostConfig.NetworkMode}}",
        ])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let id = s.strip_prefix("container:")?;
    container_name(id).or_else(|| Some(id.to_string()))
}

fn container_name(id_or_name: &str) -> Option<String> {
    let out = Command::new("docker")
        .args(["inspect", id_or_name, "--format", "{{.Name}}"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Some(s.trim_start_matches('/').to_string())
}

fn published_host_ports(container: &str) -> HashSet<u16> {
    let Ok(out) = Command::new("docker").args(["port", container]).output() else {
        return HashSet::new();
    };
    if !out.status.success() {
        return HashSet::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.rsplit(':').next()?.trim().parse::<u16>().ok())
        .collect()
}

fn via_host_listening_ports(container: &str) -> HashSet<u16> {
    let Ok(out) = Command::new("docker")
        .args(["exec", container, "cat", "/proc/net/tcp"])
        .output()
    else {
        return HashSet::new();
    };
    if !out.status.success() {
        return HashSet::new();
    }
    let bridge_ip = crate::docker::bridge_gateway();
    let bridge_hex = ip_to_hex(&bridge_ip);
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let _sl = it.next()?;
            let local = it.next()?;
            let _rem = it.next()?;
            let st = it.next()?;
            if st != "0A" {
                return None;
            }
            let mut parts = local.split(':');
            let ip_hex = parts.next()?;
            let port_hex = parts.next()?;
            // socat is host-net and bind=<bridge_gw>; only count ports bound to that IP.
            if let Some(expected) = bridge_hex.as_deref()
                && !ip_hex.eq_ignore_ascii_case(expected)
            {
                return None;
            }
            u16::from_str_radix(port_hex, 16).ok()
        })
        .collect()
}

fn ip_to_hex(ip: &str) -> Option<String> {
    let mut octets = [0u8; 4];
    for (i, part) in ip.split('.').enumerate() {
        if i >= 4 {
            return None;
        }
        octets[i] = part.parse().ok()?;
    }
    // /proc/net/tcp stores the IP as little-endian hex of the 4 octets,
    // i.e. octets reversed and hex-formatted upper-case.
    Some(format!(
        "{:02X}{:02X}{:02X}{:02X}",
        octets[3], octets[2], octets[1], octets[0]
    ))
}

fn listening_ports(container: &str) -> HashSet<u16> {
    let Ok(out) = Command::new("docker")
        .args(["exec", container, "cat", "/proc/net/tcp"])
        .output()
    else {
        return HashSet::new();
    };
    if !out.status.success() {
        return HashSet::new();
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut it = line.split_whitespace();
            let _sl = it.next()?;
            let local = it.next()?;
            let _rem = it.next()?;
            let st = it.next()?;
            if st != "0A" {
                return None;
            }
            let port_hex = local.split(':').nth(1)?;
            u16::from_str_radix(port_hex, 16).ok()
        })
        .collect()
}

fn logs(project_root: &Path, follow: bool) {
    let pname = project_name(project_root);
    let sidecar = tunnel::sidecar_name(&pname);
    let via_host = tunnel::via_host_sidecar_name(&pname);
    let in_netns_present = tunnel::sidecar_exists(&sidecar);
    let via_host_present = tunnel::via_host_sidecar_exists(&via_host);
    if !in_netns_present && !via_host_present {
        log(format!("no tunnel sidecars present for {pname}"));
        return;
    }
    if in_netns_present {
        crate::docker::tail_logs(&sidecar, follow);
    }
    if via_host_present {
        crate::docker::tail_logs(&via_host, follow);
    }
}

fn stop(project_root: &Path) {
    let pname = project_name(project_root);
    let sidecar = tunnel::sidecar_name(&pname);
    let via_host = tunnel::via_host_sidecar_name(&pname);
    let in_netns_present = tunnel::sidecar_exists(&sidecar);
    let via_host_present = tunnel::via_host_sidecar_exists(&via_host);
    if !in_netns_present && !via_host_present {
        log(format!("no tunnel sidecars present for {pname}"));
        return;
    }
    if in_netns_present {
        tunnel::stop_sidecar(&sidecar);
    }
    if via_host_present {
        tunnel::stop_via_host_sidecar(&via_host);
    }
}

pub fn run(cwd: &Path, action: Action<'_>) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));

    match action {
        Action::List => {
            let cfg = Config::load_or_default(&root);
            if cfg.tunnels.is_empty() {
                log("no tunnels configured");
                return;
            }
            for t in &cfg.tunnels {
                println!("{}: {} = {}", t.dir, t.left, t.right.as_string());
            }
        }
        Action::Add {
            direction,
            left,
            right,
        } => {
            let Some(dir) = Direction::parse(direction) else {
                die(format!(
                    "invalid direction: {direction} (use out, in, via, or via-host)"
                ));
            };
            let Ok(left_port) = left.parse::<u16>() else {
                die(format!("invalid port: {left}"));
            };
            let right_val = match dir {
                Direction::Out | Direction::In => match right.parse::<u16>() {
                    Ok(p) => TunnelRight::Port(p),
                    Err(_) => die(format!("invalid right-side port: {right}")),
                },
                Direction::Via | Direction::ViaHost => {
                    let Some((host, port)) = right.rsplit_once(':') else {
                        die(format!(
                            "{} right side must be host:port, got: {right}",
                            dir.as_str()
                        ));
                    };
                    if host.is_empty() || port.parse::<u16>().is_err() {
                        die(format!(
                            "{} right side must be host:port, got: {right}",
                            dir.as_str()
                        ));
                    }
                    TunnelRight::Address(right.to_string())
                }
            };
            let cfg = Config::load_or_default(&root);
            if cfg
                .tunnels
                .iter()
                .any(|t| t.dir == dir.as_str() && t.left == left_port)
            {
                die(format!(
                    "{} {} already configured",
                    dir.as_str(),
                    left_port,
                ));
            }
            let path = Config::edit(&root, |c| {
                c.tunnels.push(ConfigTunnel {
                    dir: dir.as_str().to_string(),
                    left: left_port,
                    right: right_val,
                });
            })
            .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!(
                "added {} {} = {} in {}",
                dir.as_str(),
                left_port,
                right,
                path.display()
            ));
        }
        Action::Remove { direction, left } => {
            let Some(dir) = Direction::parse(direction) else {
                die(format!(
                    "invalid direction: {direction} (use out, in, via, or via-host)"
                ));
            };
            let Ok(left_port) = left.parse::<u16>() else {
                die(format!("invalid port: {left}"));
            };
            let path = Config::edit(&root, |c| {
                c.tunnels
                    .retain(|t| !(t.dir == dir.as_str() && t.left == left_port));
            })
            .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!(
                "removed {} {} from {}",
                dir.as_str(),
                left_port,
                path.display()
            ));
        }
    }
}
