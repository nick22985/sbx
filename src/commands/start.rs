use std::path::Path;

use crate::config::Config;
use crate::project::project_flavor;
use crate::util::{die, log};

pub enum Action<'a> {
    Show,
    Set(&'a [String]),
    Clear,
}

pub fn run(cwd: &Path, action: Action<'_>) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));
    match action {
        Action::Show => match Config::load_or_default(&root).start {
            Some(s) if !s.trim().is_empty() => println!("{s}"),
            _ => log("no start command configured"),
        },
        Action::Set(cmd) => {
            if cmd.is_empty() {
                die("usage: sbx start set <command...>");
            }
            let joined = cmd.join(" ");
            let path = Config::edit(&root, |c| c.start = Some(joined.clone()))
                .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!("set start in {}: {joined}", path.display()));
        }
        Action::Clear => {
            let had = Config::load_or_default(&root).start.is_some();
            let path = Config::edit(&root, |c| c.start = None)
                .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            if had {
                log(format!("cleared start in {}", path.display()));
            } else {
                log("no start command configured");
            }
        }
    }
}

pub fn write_raw(cwd: &Path, raw: &str) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));
    let path = Config::edit(&root, |c| c.start = Some(raw.to_string()))
        .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
    log(format!("set start in {}: {raw}", path.display()));
}
