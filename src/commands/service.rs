use std::path::Path;

use crate::config::Config;
use crate::project::project_flavor;
use crate::service::{BUILTIN_SERVICES, project_services, service_template};
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
            let svcs = project_services(&root);
            if svcs.is_empty() {
                log("no services configured");
                return;
            }
            for s in svcs {
                println!("{s}");
            }
        }
        Action::Add(name) => {
            if service_template(name).is_none() {
                die(format!(
                    "unknown service: '{name}' (built-ins: {}; or pass an image like 'ghcr.io/foo/bar:tag')",
                    BUILTIN_SERVICES.join(" ")
                ));
            }
            if Config::load_or_default(&root)
                .services
                .enabled
                .iter()
                .any(|s| s == name)
            {
                log(format!("already present: {name}"));
                return;
            }
            let path = Config::edit(&root, |c| {
                c.services.enabled.push(name.to_string());
            })
            .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!("added {name} to {}", path.display()));
        }
        Action::Remove(name) => {
            let path = Config::edit(&root, |c| {
                c.services.enabled.retain(|s| s != name);
            })
            .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!("removed {name} from {}", path.display()));
        }
    }
}
