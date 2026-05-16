use std::collections::BTreeSet;
use std::path::Path;

use crate::util::config_lines;

/// Write a project's fragment of lines into `dir/pname`. Empty lines slice
/// removes the file. Lines are written one per line (no comments).
pub fn write(dir: &Path, pname: &str, lines: &[String]) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let path = dir.join(pname);
    if lines.is_empty() {
        let _ = std::fs::remove_file(&path);
        return Ok(());
    }
    let body: String = lines.iter().map(|h| format!("{h}\n")).collect();
    std::fs::write(&path, body).map_err(|e| format!("write {}: {e}", path.display()))
}

pub fn remove(dir: &Path, pname: &str) {
    let _ = std::fs::remove_file(dir.join(pname));
}

/// Read every file in `dir`, parse each via `config_lines`, dedup, and return
/// sorted lines. Missing `dir` yields an empty Vec.
pub fn merged(dir: &Path) -> Vec<String> {
    let mut all: BTreeSet<String> = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    for e in entries.flatten() {
        let Ok(body) = std::fs::read_to_string(e.path()) else {
            continue;
        };
        for line in config_lines(&body) {
            all.insert(line.to_string());
        }
    }
    all.into_iter().collect()
}
