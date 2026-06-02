use crate::config::GlobalConfig;
use crate::util::{die, log};

pub enum Action {
    On,
    Off,
    Status,
}

pub fn run(action: Action) {
    match action {
        Action::On => {
            let mut cfg = GlobalConfig::load_or_default();
            cfg.clipboard = true;
            let path = cfg
                .save()
                .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log(format!(
                "clipboard forwarding enabled for ALL flavors ({})",
                path.display()
            ));
            log(
                "for a single flavor instead, set `clipboard = true` in that flavor's config.toml (e.g. nvim)",
            );
            log(
                "next container start will forward the host Wayland socket so wl-copy/wl-paste reach the host clipboard",
            );
        }
        Action::Off => {
            let mut cfg = GlobalConfig::load_or_default();
            cfg.clipboard = false;
            let _ = cfg
                .save()
                .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
            log("clipboard forwarding disabled");
        }
        Action::Status => {
            if GlobalConfig::load_or_default().clipboard {
                log("clipboard forwarding: ON (global, all flavors)");
                let wayland = std::env::var("WAYLAND_DISPLAY").ok();
                let rt = std::env::var("XDG_RUNTIME_DIR").ok();
                match (&wayland, &rt) {
                    (Some(w), Some(r)) if !w.is_empty() && !r.is_empty() => {
                        log(format!("host XDG_RUNTIME_DIR={r} WAYLAND_DISPLAY={w}"))
                    }
                    _ => log("host: no WAYLAND_DISPLAY/XDG_RUNTIME_DIR detected"),
                }
            } else {
                log("clipboard forwarding: off");
            }
        }
    }
}
