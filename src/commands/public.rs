use std::fs;
use std::path::Path;

use crate::project::{project_flavor, project_name, sbx_file, sbx_write_dir};
use crate::public::{self, PublicRoute, SIDECAR, parse_public};
use crate::util::{die, log};

pub enum Action<'a> {
    List,
    Add(&'a str, &'a str),
    Remove(&'a str),
    Login,
    Status,
    Logs { follow: bool },
    Stop,
}

pub fn run(cwd: &Path, action: Action<'_>) {
    match action {
        Action::List => list(cwd),
        Action::Add(host, port) => add(cwd, host, port),
        Action::Remove(host) => remove(cwd, host),
        Action::Login => login(),
        Action::Status => status(cwd),
        Action::Logs { follow } => logs(follow),
        Action::Stop => stop(),
    }
}

fn require_project(cwd: &Path) -> std::path::PathBuf {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
    root
}

fn list(cwd: &Path) {
    let root = require_project(cwd);
    let read_file = sbx_file(&root, "public");
    let content = fs::read_to_string(&read_file).unwrap_or_default();
    if content.trim().is_empty() {
        log("no public hostnames configured");
        return;
    }
    log(format!("from {}:", read_file.display()));
    print!("{content}");
    if !content.ends_with('\n') {
        println!();
    }
}

fn add(cwd: &Path, host: &str, port: &str) {
    if port.parse::<u16>().is_err() {
        die(format!("invalid port: {port}"));
    }
    if host.is_empty() || host.contains(char::is_whitespace) {
        die(format!("invalid hostname: {host}"));
    }
    let root = require_project(cwd);
    let write_dir = sbx_write_dir(&root);
    let write_file = write_dir.join("public");
    fs::create_dir_all(&write_dir).ok();
    let mut content = fs::read_to_string(&write_file).unwrap_or_default();
    let existing: Vec<PublicRoute> = parse_public(&content);
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

fn remove(cwd: &Path, host: &str) {
    let root = require_project(cwd);
    let write_dir = sbx_write_dir(&root);
    let write_file = write_dir.join("public");
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

fn login() {
    if let Err(e) = public::login() {
        die(e);
    }
    log("cloudflare login complete. next: cd into a project and 'sbx public add HOST=PORT'");
}

fn status(cwd: &Path) {
    let logged = public::logged_in();
    let tunnel = public::tunnel_exists();

    println!("sidecar:   {}", crate::docker::state_line(SIDECAR));
    println!(
        "login:     {}  ({})",
        if logged { "ok" } else { "missing" },
        public::cert_pem().display()
    );
    println!(
        "tunnel:    {}  ({})",
        if tunnel { "created" } else { "missing" },
        public::credentials_file().display()
    );

    let merged = public::merged_hostnames();
    println!(
        "merged:    {} hostname(s) across active sessions",
        merged.len()
    );
    for h in &merged {
        println!("  - {h}");
    }

    if let Some((_, root)) = project_flavor(cwd) {
        let local: Vec<_> = public::read_public(&root)
            .into_iter()
            .map(|r| format!("{}  ->  :{}", r.hostname, r.port))
            .collect();
        println!(
            "project:   {} ({} entr{} in .sbx/public)",
            project_name(&root),
            local.len(),
            if local.len() == 1 { "y" } else { "ies" }
        );
        for l in &local {
            println!("  - {l}");
        }
    }
}

fn logs(follow: bool) {
    if !public::sidecar_exists() {
        log(format!("cloudflared sidecar not present ({SIDECAR})"));
        return;
    }
    crate::docker::tail_logs(SIDECAR, follow);
}

fn stop() {
    if public::force_stop() {
        log(format!("stopped cloudflared sidecar: {SIDECAR}"));
    } else {
        log(format!("cloudflared sidecar not present ({SIDECAR})"));
    }
}
