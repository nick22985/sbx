use std::path::Path;

use crate::config::Config;
use crate::project::project_flavor;
use crate::util::{die, log};

pub enum Action<'a> {
    List,
    Add(&'a str),
    Remove(&'a str),
}

pub fn run(cwd: &Path, action: Action<'_>) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));

    match action {
        Action::List => {
            let cfg = Config::load_or_default(&root);
            if cfg.ports.is_empty() {
                log("no ports configured");
                return;
            }
            for p in &cfg.ports {
                println!("{p}");
            }
        }
        Action::Add(p) => {
            let Ok(n) = p.parse::<u16>() else {
                die(format!("invalid port: {p}"));
            };
            if Config::load_or_default(&root).ports.contains(&n) {
                log(format!("port {p} already present"));
                return;
            }
            let path = Config::edit(&root, |c| {
                c.ports.push(n);
            })
            .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!("added {p} to {}", path.display()));
        }
        Action::Remove(p) => {
            let Ok(n) = p.parse::<u16>() else {
                die(format!("invalid port: {p}"));
            };
            let path = Config::edit(&root, |c| {
                c.ports.retain(|x| *x != n);
            })
            .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!("removed {p} from {}", path.display()));
        }
    }
}
