use std::path::Path;

use crate::config::Config;
use crate::project::{project_flavor, project_name};
use crate::public::{self, SIDECAR};
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
        .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));
    root
}

fn list(cwd: &Path) {
    let root = require_project(cwd);
    let cfg = Config::load_or_default(&root);
    if cfg.public.is_empty() {
        log("no public hostnames configured");
        return;
    }
    for (h, p) in &cfg.public {
        println!("{h} = {p}");
    }
}

fn add(cwd: &Path, host: &str, port: &str) {
    let Ok(port_n) = port.parse::<u16>() else {
        die(format!("invalid port: {port}"));
    };
    if host.is_empty() || host.contains(char::is_whitespace) {
        die(format!("invalid hostname: {host}"));
    }
    let root = require_project(cwd);
    if Config::load_or_default(&root).public.contains_key(host) {
        die(format!("hostname {host} already mapped"));
    }
    let path = Config::edit(&root, |c| {
        c.public.insert(host.to_string(), port_n);
    })
    .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
    log(format!("added {host} -> :{port} in {}", path.display()));
}

fn remove(cwd: &Path, host: &str) {
    let root = require_project(cwd);
    let path = Config::edit(&root, |c| {
        c.public.remove(host);
    })
    .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
    log(format!("removed {host} from {}", path.display()));
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
            "project:   {} ({} public entr{} in config.toml)",
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
