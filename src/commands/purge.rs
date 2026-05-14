use std::process::{Command, Stdio};

use crate::flavor::flavor_volumes;
use crate::util::{confirm, die, log};

pub fn run(flavor: Option<&str>) {
    let flavors: Vec<&str> = match flavor {
        Some(f) => match f {
            "npm" | "bun" | "rust" | "claude" => vec![f],
            other => die(format!("unknown flavor: {other}")),
        },
        None => vec!["npm", "bun", "rust", "claude"],
    };
    let mut vols: Vec<String> = Vec::new();
    let mut imgs: Vec<String> = Vec::new();
    for f in &flavors {
        for v in flavor_volumes(f) {
            vols.push(v.to_string());
        }
        imgs.extend(images_for_flavor(f));
    }
    if vols.is_empty() && imgs.is_empty() {
        log(format!("nothing to purge for: {}", flavors.join(" ")));
        return;
    }
    eprintln!("sbx: will purge:");
    for i in &imgs {
        eprintln!("  image: {i}");
    }
    for v in &vols {
        eprintln!("  volume: {v}");
    }
    if !confirm("continue?") {
        log("aborted");
        return;
    }
    for i in &imgs {
        log(format!("removing image {i}"));
        let ok = Command::new("docker")
            .args(["image", "rm", i])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            log("  (failed; in use?)");
        }
    }
    for v in &vols {
        if !volume_exists(v) {
            continue;
        }
        log(format!("removing volume {v}"));
        let _ = Command::new("docker")
            .args(["volume", "rm", v])
            .stdout(Stdio::null())
            .status();
    }
}

fn images_for_flavor(flavor: &str) -> Vec<String> {
    let Ok(out) = Command::new("docker")
        .args(["images", "--format", "{{.Repository}}:{{.Tag}}"])
        .output()
    else {
        return Vec::new();
    };
    let needle_prefix = format!("sbx-{flavor}");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let (repo, _tag) = line.split_once(':')?;
            if repo == needle_prefix || repo.starts_with(&format!("{needle_prefix}-")) {
                Some(line.to_string())
            } else {
                None
            }
        })
        .collect()
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
