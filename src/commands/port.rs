use std::fs;
use std::path::Path;

use crate::project::{project_flavor, sbx_file, sbx_write_dir};
use crate::util::{die, log};

pub enum Action<'a> {
    List,
    Add(&'a str),
    Remove(&'a str),
}

pub fn run(cwd: &Path, action: Action<'_>) {
    let (_, root) =
        project_flavor(cwd).unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
    let read_file = sbx_file(&root, "ports");
    let write_dir = sbx_write_dir(&root);
    let write_file = write_dir.join("ports");

    match action {
        Action::List => {
            let content = fs::read_to_string(&read_file).unwrap_or_default();
            if content.trim().is_empty() {
                log("no ports configured");
                return;
            }
            log(format!("from {}:", read_file.display()));
            print!("{content}");
            if !content.ends_with('\n') {
                println!();
            }
        }
        Action::Add(p) => {
            if p.parse::<u16>().is_err() {
                die(format!("invalid port: {p}"));
            }
            fs::create_dir_all(&write_dir).ok();
            let mut content = fs::read_to_string(&write_file).unwrap_or_default();
            for line in content.lines() {
                let cleaned: String =
                    line.split('#').next().unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect();
                if cleaned == p {
                    log(format!("port {p} already present in {}", write_file.display()));
                    return;
                }
            }
            if !content.ends_with('\n') && !content.is_empty() {
                content.push('\n');
            }
            content.push_str(p);
            content.push('\n');
            if let Err(e) = fs::write(&write_file, content) {
                die(format!("write {}: {e}", write_file.display()));
            }
            log(format!("added {p} to {}", write_file.display()));
        }
        Action::Remove(p) => {
            if !write_file.is_file() {
                die(format!("no {}", write_file.display()));
            }
            let content = fs::read_to_string(&write_file).unwrap_or_default();
            let kept: Vec<&str> = content
                .lines()
                .filter(|line| {
                    let cleaned: String = line
                        .split('#')
                        .next()
                        .unwrap_or("")
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect();
                    cleaned != p
                })
                .collect();
            let mut out = kept.join("\n");
            if !out.is_empty() {
                out.push('\n');
            }
            if let Err(e) = fs::write(&write_file, out) {
                die(format!("write {}: {e}", write_file.display()));
            }
            log(format!("removed {p} from {}", write_file.display()));
        }
    }
}
