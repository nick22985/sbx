use std::path::Path;

use crate::config::Config;
use crate::docker::project_gui_enabled;
use crate::project::project_flavor;
use crate::util::{die, log};

pub enum Action {
    On,
    Off,
    Status,
}

pub fn run(cwd: &Path, action: Action) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));
    match action {
        Action::On => {
            let path = Config::edit(&root, |c| c.gui = true)
                .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!(
                "GUI forwarding enabled for this project ({})",
                path.display()
            ));
            log(
                "next container start will mount the Wayland + X11 sockets and forward DISPLAY/WAYLAND_DISPLAY/XDG_RUNTIME_DIR",
            );
        }
        Action::Off => {
            let _ = Config::edit(&root, |c| c.gui = false);
            log("GUI forwarding disabled");
        }
        Action::Status => {
            if project_gui_enabled(&root) {
                log("GUI forwarding: ON");
                let wayland = std::env::var("WAYLAND_DISPLAY").ok();
                let rt = std::env::var("XDG_RUNTIME_DIR").ok();
                let display = std::env::var("DISPLAY").ok();
                match (&wayland, &rt) {
                    (Some(w), Some(r)) if !w.is_empty() && !r.is_empty() => {
                        log(format!("host XDG_RUNTIME_DIR={r} WAYLAND_DISPLAY={w}"))
                    }
                    _ => log("host: no WAYLAND_DISPLAY/XDG_RUNTIME_DIR detected"),
                }
                match &display {
                    Some(d) if !d.is_empty() => log(format!("host DISPLAY={d}")),
                    _ => log("host: no DISPLAY detected"),
                }
            } else {
                log("GUI forwarding: off");
            }
        }
    }
}
