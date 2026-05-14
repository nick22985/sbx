use std::fs;
use std::path::Path;

use crate::project::{project_flavor, sbx_file, sbx_write_dir};
use crate::util::{die, log};

pub enum Action<'a> {
    Show,
    Set(&'a [String]),
    Clear,
}

pub fn run(cwd: &Path, action: Action<'_>) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
    let read_file = sbx_file(&root, "start");
    let write_dir = sbx_write_dir(&root);
    let write_file = write_dir.join("start");

    match action {
        Action::Show => {
            if !read_file.is_file() {
                log("no start file");
                return;
            }
            log(format!("from {}:", read_file.display()));
            let content = fs::read_to_string(&read_file).unwrap_or_default();
            print!("{content}");
            if !content.ends_with('\n') {
                println!();
            }
        }
        Action::Set(cmd) => {
            if cmd.is_empty() {
                die("usage: sbx start set <command...>");
            }
            if let Err(e) = fs::create_dir_all(&write_dir) {
                die(format!("mkdir {}: {e}", write_dir.display()));
            }
            let joined = cmd.join(" ");
            if let Err(e) = fs::write(&write_file, format!("{joined}\n")) {
                die(format!("write {}: {e}", write_file.display()));
            }
            log(format!("wrote {}: {joined}", write_file.display()));
        }
        Action::Clear => {
            let target = if write_file.is_file() {
                Some(write_file.clone())
            } else if read_file.is_file() {
                Some(read_file.clone())
            } else {
                None
            };
            match target {
                Some(p) => {
                    if let Err(e) = fs::remove_file(&p) {
                        die(format!("remove {}: {e}", p.display()));
                    }
                    log(format!("removed {}", p.display()));
                }
                None => log("no start file"),
            }
        }
    }
}

pub fn write_raw(cwd: &Path, raw: &str) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
    let write_dir = sbx_write_dir(&root);
    let write_file = write_dir.join("start");
    if let Err(e) = fs::create_dir_all(&write_dir) {
        die(format!("mkdir {}: {e}", write_dir.display()));
    }
    let body = format!("{raw}\n");
    if let Err(e) = fs::write(&write_file, &body) {
        die(format!("write {}: {e}", write_file.display()));
    }
    log(format!("wrote {}: {raw}", write_file.display()));
}
