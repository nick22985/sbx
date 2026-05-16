use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::docker;
use crate::project::{project_flavor, project_name, sbx_file, sbx_write_dir};
use crate::tunnel::{self, Direction, parse};
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
        .unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
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
        })
        .collect();
    let arrow_w = arrows.iter().map(|a| a.len()).max().unwrap_or(0);
    for (t, arrow) in tunnels.iter().zip(arrows.iter()) {
        let state = tunnel_state(t, main.is_some(), &published, &sidecar_listening);
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
) -> &'static str {
    if !session_running {
        return "down";
    }
    match t.direction {
        Direction::Out => {
            let hp = t.right.parse::<u16>().unwrap_or(t.left);
            if published.contains(&hp) { "active" } else { "down" }
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
    if !tunnel::sidecar_exists(&sidecar) {
        log(format!("tunnel sidecar not present ({sidecar})"));
        return;
    }
    crate::docker::tail_logs(&sidecar, follow);
}

fn stop(project_root: &Path) {
    let pname = project_name(project_root);
    let sidecar = tunnel::sidecar_name(&pname);
    if !tunnel::sidecar_exists(&sidecar) {
        log(format!("tunnel sidecar not present ({sidecar})"));
        return;
    }
    tunnel::stop_sidecar(&sidecar);
}

pub fn run(cwd: &Path, action: Action<'_>) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
    let read_file = sbx_file(&root, "tunnels");
    let write_dir = sbx_write_dir(&root);
    let write_file = write_dir.join("tunnels");

    match action {
        Action::List => {
            let content = fs::read_to_string(&read_file).unwrap_or_default();
            if content.trim().is_empty() {
                log("no tunnels configured");
                return;
            }
            log(format!("from {}:", read_file.display()));
            print!("{content}");
            if !content.ends_with('\n') {
                println!();
            }
        }
        Action::Add {
            direction,
            left,
            right,
        } => {
            let Some(dir) = Direction::parse(direction) else {
                die(format!(
                    "invalid direction: {direction} (use out, in, or via)"
                ));
            };
            let Ok(left_port) = left.parse::<u16>() else {
                die(format!("invalid port: {left}"));
            };
            match dir {
                Direction::Out | Direction::In => {
                    if right.parse::<u16>().is_err() {
                        die(format!("invalid right-side port: {right}"));
                    }
                }
                Direction::Via => {
                    let Some((host, port)) = right.rsplit_once(':') else {
                        die(format!("via right side must be host:port, got: {right}"));
                    };
                    if host.is_empty() || port.parse::<u16>().is_err() {
                        die(format!("via right side must be host:port, got: {right}"));
                    }
                }
            }

            fs::create_dir_all(&write_dir).ok();
            let mut content = fs::read_to_string(&write_file).unwrap_or_default();
            let existing = parse(&content);
            if existing
                .iter()
                .any(|t| t.direction == dir && t.left == left_port)
            {
                die(format!(
                    "{} {} already configured in {}",
                    dir.as_str(),
                    left_port,
                    write_file.display()
                ));
            }
            if !content.ends_with('\n') && !content.is_empty() {
                content.push('\n');
            }
            content.push_str(&format!("{}: {} = {}\n", dir.as_str(), left_port, right));
            if let Err(e) = fs::write(&write_file, content) {
                die(format!("write {}: {e}", write_file.display()));
            }
            log(format!(
                "added {} {} = {} in {}",
                dir.as_str(),
                left_port,
                right,
                write_file.display()
            ));
        }
        Action::Remove { direction, left } => {
            let Some(dir) = Direction::parse(direction) else {
                die(format!(
                    "invalid direction: {direction} (use out, in, or via)"
                ));
            };
            let Ok(left_port) = left.parse::<u16>() else {
                die(format!("invalid port: {left}"));
            };
            if !write_file.is_file() {
                die(format!("no {}", write_file.display()));
            }
            let content = fs::read_to_string(&write_file).unwrap_or_default();
            let kept: Vec<&str> = content
                .lines()
                .filter(|line| {
                    let body = line.split('#').next().unwrap_or("").trim();
                    let Some((d, rest)) = body.split_once(':') else {
                        return true;
                    };
                    let Some((l, _)) = rest.split_once('=') else {
                        return true;
                    };
                    let parsed_dir = Direction::parse(d);
                    let parsed_left = l.trim().parse::<u16>().ok();
                    !(parsed_dir == Some(dir) && parsed_left == Some(left_port))
                })
                .collect();
            let mut out = kept.join("\n");
            if !out.is_empty() {
                out.push('\n');
            }
            if let Err(e) = fs::write(&write_file, out) {
                die(format!("write {}: {e}", write_file.display()));
            }
            log(format!(
                "removed {} {} from {}",
                dir.as_str(),
                left_port,
                write_file.display()
            ));
        }
    }
}
