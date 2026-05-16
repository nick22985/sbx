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
    if g == c || !c.join("worktrees").is_dir() {
        return None;
    }
    if c.file_name().and_then(|n| n.to_str()) == Some(".git")
        && let Some(parent) = c.parent()
    {
        return Some(parent.to_path_buf());
    }
    Some(c)
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

pub fn project_base_name(root: &Path) -> String {
    let base = shared_root(root).unwrap_or_else(|| root.to_path_buf());
    let last = base
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sbx".to_string());
    sanitize_tag(&last)
}

pub fn worktree_suffix(root: &Path) -> Option<String> {
    if shared_root(root).is_none() {
        return None;
    }
    let name_file = root.join(".sbx/name");
    if let Ok(content) = fs::read_to_string(&name_file) {
        let s = sanitize_tag(content.trim());
        if !s.is_empty() {
            return Some(s);
        }
    }
    if let Some(branch) = current_branch(root) {
        let s = sanitize_tag(&branch);
        if !s.is_empty() {
            return Some(s);
        }
    }
    let wt_name = root
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let wt_sanitized = sanitize_tag(&wt_name);
    if !wt_sanitized.is_empty() && wt_sanitized != project_base_name(root) {
        return Some(wt_sanitized);
    }
    None
}

pub fn project_name(root: &Path) -> String {
    let base = project_base_name(root);
    match worktree_suffix(root) {
        Some(s) => format!("{base}-{s}"),
        None => base,
    }
}

/// Per-worktree port shift, so multiple worktrees sharing one VPN sidecar
/// (i.e. one network namespace) can each bind their declared listen ports
/// without colliding. `master` / `main` / non-worktree always return 0;
/// any other worktree gets a stable hash-derived offset in [1, 9].
///
/// Override per worktree by writing a number to `.sbx/port-offset`.
pub fn port_offset(root: &Path) -> u16 {
    let pin = root.join(".sbx/port-offset");
    if let Ok(content) = fs::read_to_string(&pin)
        && let Ok(n) = content.trim().parse::<u16>()
    {
        return n;
    }
    let Some(suffix) = worktree_suffix(root) else {
        return 0;
    };
    if matches!(suffix.as_str(), "master" | "main") {
        return 0;
    }
    let mut h: u32 = 5381;
    for b in suffix.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u32);
    }
    1 + (h % 9) as u16
}

/// Implemented by route-like types that get adjusted when reading from a
/// worktree: `hostname` is prefixed with the suffix, `port` gets `port_offset`.
pub trait WorktreeAdjustable {
    fn hostname_mut(&mut self) -> &mut String;
    fn port_mut(&mut self) -> &mut u16;
}

/// Apply the worktree's hostname-prefix and port-offset to every item.
pub fn apply_worktree_remap<T: WorktreeAdjustable>(root: &Path, items: &mut [T]) {
    if let Some(suffix) = worktree_suffix(root) {
        for it in items.iter_mut() {
            let h = it.hostname_mut();
            *h = format!("{suffix}-{h}");
        }
    }
    let offset = port_offset(root);
    if offset > 0 {
        for it in items.iter_mut() {
            let p = it.port_mut();
            *p = p.saturating_add(offset);
        }
    }
}

fn current_branch(root: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
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

    fn tmp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("sbx-test-{label}-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn run_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&status.stderr)
        );
    }

    fn make_repo_with_worktree(label: &str) -> (PathBuf, PathBuf, PathBuf) {
        let root = tmp_dir(label);
        let main = root.join("myapp");
        std::fs::create_dir_all(&main).unwrap();
        run_git(&main, &["init", "-q", "-b", "master"]);
        std::fs::write(main.join("README"), "x").unwrap();
        run_git(&main, &["add", "."]);
        run_git(&main, &["commit", "-q", "-m", "init"]);
        let wt = root.join("myapp-wt");
        run_git(
            &main,
            &["worktree", "add", "-b", "feature/foo", wt.to_str().unwrap()],
        );
        (root, main, wt)
    }

    #[test]
    fn project_name_unchanged_in_main_checkout() {
        let (_root, main, _wt) = make_repo_with_worktree("main-check");
        assert_eq!(project_base_name(&main), "myapp");
        assert_eq!(worktree_suffix(&main), None);
        assert_eq!(project_name(&main), "myapp");
    }

    #[test]
    fn project_name_includes_branch_in_worktree() {
        let (_root, _main, wt) = make_repo_with_worktree("wt-branch");
        assert_eq!(project_base_name(&wt), "myapp");
        assert_eq!(worktree_suffix(&wt).as_deref(), Some("feature-foo"));
        assert_eq!(project_name(&wt), "myapp-feature-foo");
    }

    #[test]
    fn sbx_name_overrides_branch_in_worktree() {
        let (_root, _main, wt) = make_repo_with_worktree("wt-name");
        std::fs::create_dir_all(wt.join(".sbx")).unwrap();
        std::fs::write(wt.join(".sbx/name"), "exp1\n").unwrap();
        assert_eq!(worktree_suffix(&wt).as_deref(), Some("exp1"));
        assert_eq!(project_name(&wt), "myapp-exp1");
    }
}
