use std::path::Path;

use crate::config::Config;
use crate::project::project_flavor;
use crate::util::{die, log};

pub enum Action<'a> {
    List,
    Add(&'a str, &'a str),
    Remove(&'a str),
}

pub fn run(cwd: &Path, action: Action<'_>) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));

    match action {
        Action::List => {
            let cfg = Config::load_or_default(&root);
            if cfg.hostname.is_empty() {
                log("no hostnames configured");
                return;
            }
            for (h, p) in &cfg.hostname {
                println!("{h} = {p}");
            }
        }
        Action::Add(host, port) => {
            let Ok(port_n) = port.parse::<u16>() else {
                die(format!("invalid port: {port}"));
            };
            if host.is_empty() || host.contains(char::is_whitespace) {
                die(format!("invalid hostname: {host}"));
            }
            if Config::load_or_default(&root).hostname.contains_key(host) {
                die(format!("hostname {host} already mapped"));
            }
            let path = Config::edit(&root, |c| {
                c.hostname.insert(host.to_string(), port_n);
            })
            .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!("added {host} -> :{port} in {}", path.display()));
        }
        Action::Remove(host) => {
            let path = Config::edit(&root, |c| {
                c.hostname.remove(host);
            })
            .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!("removed {host} from {}", path.display()));
        }
    }
}
