use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::util::{home_dir, private_dir, sanitize_tag};

pub fn shared_root(dir: &Path) -> Option<PathBuf> {
    let git_dir = git_rev_parse(dir, "--git-dir")?;
    let common_dir = git_rev_parse(dir, "--git-common-dir")?;
    let resolve = |p: PathBuf| -> Option<PathBuf> {
        let abs = if p.is_absolute() { p } else { dir.join(p) };
        fs::canonicalize(&abs).ok()
    };
    let g = resolve(git_dir)?;
    let c = resolve(common_dir)?;
    if g != c && c.join("worktrees").is_dir() {
        Some(c)
    } else {
        None
    }
}

fn git_rev_parse(dir: &Path, arg: &str) -> Option<PathBuf> {
    let out = Command::new("git")
        .args(["-C"])
        .arg(dir)
        .args(["rev-parse", arg])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(PathBuf::from(s))
    }
}

fn private_key(base: &Path) -> String {
    let home = home_dir();
    if let Ok(rest) = base.strip_prefix(&home) {
        return rest.to_string_lossy().into_owned();
    }
    let s = base.to_string_lossy();
    s.trim_start_matches('/').to_string()
}

pub fn private_sbx_dir(root: &Path) -> Option<PathBuf> {
    let pd = private_dir();
    if !pd.is_dir() {
        return None;
    }
    let base = shared_root(root).unwrap_or_else(|| root.to_path_buf());
    Some(pd.join(private_key(&base)).join(".sbx"))
}

pub fn sbx_file(root: &Path, name: &str) -> PathBuf {
    let local = root.join(".sbx").join(name);
    if local.exists() {
        return local;
    }
    if let Some(shared) = shared_root(root) {
        let p = shared.join(".sbx").join(name);
        if p.exists() {
            return p;
        }
    }
    if let Some(priv_dir) = private_sbx_dir(root) {
        let p = priv_dir.join(name);
        if p.exists() {
            return p;
        }
    }
    local
}

pub fn sbx_write_dir(root: &Path) -> PathBuf {
    if root.join(".sbx/flavor").is_file() {
        return root.join(".sbx");
    }
    let shared = shared_root(root);
    if let Some(s) = &shared
        && s.join(".sbx/flavor").is_file()
    {
        return s.join(".sbx");
    }
    if let Some(priv_dir) = private_sbx_dir(root)
        && priv_dir.join("flavor").is_file()
    {
        return priv_dir;
    }
    if let Some(s) = shared {
        return s.join(".sbx");
    }
    root.join(".sbx")
}

pub fn project_name(root: &Path) -> String {
    let base = shared_root(root).unwrap_or_else(|| root.to_path_buf());
    let last = base
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sbx".to_string());
    sanitize_tag(&last)
}

pub fn project_flavor(start: &Path) -> Option<(String, PathBuf)> {
    let mut dir = start.to_path_buf();
    loop {
        let candidate = dir.join(".sbx/flavor");
        if candidate.is_file() {
            return Some((read_flavor(&candidate)?, dir));
        }
        if let Some(shared) = shared_root(&dir) {
            let c = shared.join(".sbx/flavor");
            if c.is_file() {
                return Some((read_flavor(&c)?, dir));
            }
        }
        if let Some(priv_dir) = private_sbx_dir(&dir) {
            let c = priv_dir.join("flavor");
            if c.is_file() {
                return Some((read_flavor(&c)?, dir));
            }
        }
        if dir.parent().is_none() || dir == Path::new("/") {
            return None;
        }
        match dir.parent() {
            Some(p) if p != dir => dir = p.to_path_buf(),
            _ => return None,
        }
    }
}

fn read_flavor(path: &Path) -> Option<String> {
    Some(fs::read_to_string(path).ok()?.trim().to_string())
}

pub fn private_write_dir(cwd: &Path) -> PathBuf {
    let base = shared_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    private_dir().join(private_key(&base)).join(".sbx")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_key_strips_home() {
        let key = private_key(&home_dir().join("projects/foo"));
        assert_eq!(key, "projects/foo");
    }

    #[test]
    fn private_key_for_root_path() {
        let key = private_key(Path::new("/var/tmp/x"));
        assert_eq!(key, "var/tmp/x");
    }
}
