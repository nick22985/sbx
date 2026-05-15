use std::fs;
use std::path::Path;

use crate::project::{project_flavor, sbx_file, sbx_write_dir};
use crate::proxy::{Route, parse_routes};
use crate::util::{die, log};

pub enum Action<'a> {
    List,
    Add(&'a str, &'a str),
    Remove(&'a str),
}

pub fn run(cwd: &Path, action: Action<'_>) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
    let read_file = sbx_file(&root, "hostname");
    let write_dir = sbx_write_dir(&root);
    let write_file = write_dir.join("hostname");

    match action {
        Action::List => {
            let content = fs::read_to_string(&read_file).unwrap_or_default();
            if content.trim().is_empty() {
                log("no hostnames configured");
                return;
            }
            log(format!("from {}:", read_file.display()));
            print!("{content}");
            if !content.ends_with('\n') {
                println!();
            }
        }
        Action::Add(host, port) => {
            if port.parse::<u16>().is_err() {
                die(format!("invalid port: {port}"));
            }
            if host.is_empty() || host.contains(char::is_whitespace) {
                die(format!("invalid hostname: {host}"));
            }
            fs::create_dir_all(&write_dir).ok();
            let mut content = fs::read_to_string(&write_file).unwrap_or_default();
            let existing: Vec<Route> = parse_routes(&content);
            if existing.iter().any(|r| r.hostname == host) {
                die(format!(
                    "hostname {host} already mapped in {}",
                    write_file.display()
                ));
            }
            if !content.ends_with('\n') && !content.is_empty() {
                content.push('\n');
            }
            content.push_str(&format!("{host} = {port}\n"));
            if let Err(e) = fs::write(&write_file, content) {
                die(format!("write {}: {e}", write_file.display()));
            }
            log(format!(
                "added {host} -> :{port} in {}",
                write_file.display()
            ));
        }
        Action::Remove(host) => {
            if !write_file.is_file() {
                die(format!("no {}", write_file.display()));
            }
            let content = fs::read_to_string(&write_file).unwrap_or_default();
            let kept: Vec<&str> = content
                .lines()
                .filter(|line| {
                    let body = line.split('#').next().unwrap_or("").trim();
                    match body.split_once('=') {
                        Some((h, _)) => h.trim() != host,
                        None => true,
                    }
                })
                .collect();
            let mut out = kept.join("\n");
            if !out.is_empty() {
                out.push('\n');
            }
            if let Err(e) = fs::write(&write_file, out) {
                die(format!("write {}: {e}", write_file.display()));
            }
            log(format!("removed {host} from {}", write_file.display()));
        }
    }
}
