use std::process::{Command, Stdio};

use crate::flavor::flavor_volumes;
use crate::util::{die, log};

pub fn run(flavor: Option<&str>) {
    let flavors: Vec<&str> = match flavor {
        Some(f) => match f {
            "npm" | "bun" | "rust" | "claude" => vec![f],
            other => die(format!("unknown flavor: {other}")),
        },
        None => vec!["npm", "bun", "rust", "claude"],
    };
    for f in flavors {
        for v in flavor_volumes(f) {
            if volume_exists(v) {
                log(format!("removing volume {v}"));
                let _ = Command::new("docker")
                    .args(["volume", "rm", v])
                    .stdout(Stdio::null())
                    .status();
            }
        }
    }
}

fn volume_exists(name: &str) -> bool {
    Command::new("docker")
        .args(["volume", "inspect", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
