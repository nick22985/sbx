use std::io::Write;
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::cell::RefCell;

#[cfg(test)]
thread_local! {
    static CONFIG_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static HOME_DIR_OVERRIDE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

pub fn home_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(p) = HOME_DIR_OVERRIDE.with(|h| h.borrow().clone()) {
        return p;
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(p) = CONFIG_DIR_OVERRIDE.with(|c| c.borrow().clone()) {
        return p;
    }
    dirs::config_dir()
        .unwrap_or_else(|| home_dir().join(".config"))
        .join("sbx")
}

#[cfg(test)]
pub struct TestPathGuard;

#[cfg(test)]
impl Drop for TestPathGuard {
    fn drop(&mut self) {
        CONFIG_DIR_OVERRIDE.with(|c| c.borrow_mut().take());
        HOME_DIR_OVERRIDE.with(|h| h.borrow_mut().take());
    }
}

#[cfg(test)]
pub fn set_test_paths(config: PathBuf, home: PathBuf) -> TestPathGuard {
    CONFIG_DIR_OVERRIDE.with(|c| *c.borrow_mut() = Some(config));
    HOME_DIR_OVERRIDE.with(|h| *h.borrow_mut() = Some(home));
    TestPathGuard
}

pub fn flavors_dir() -> PathBuf {
    config_dir().join("flavors")
}

pub fn env_file_path() -> PathBuf {
    config_dir().join("env")
}

pub fn private_dir() -> PathBuf {
    if let Ok(p) = std::env::var("SBX_PRIVATE_DIR") {
        return PathBuf::from(p);
    }
    home_dir().join("dotfiles/env/.config/.nickInstall/install/configs/private/sbx")
}

pub fn log(msg: impl AsRef<str>) {
    let _ = writeln!(std::io::stderr(), "sbx: {}", msg.as_ref());
}

pub fn die(msg: impl AsRef<str>) -> ! {
    eprintln!("sbx: {}", msg.as_ref());
    std::process::exit(1);
}

/// Iterate `.sbx/*` config-file lines: strips `#`-comments, trims, skips empties.
pub fn config_lines(body: &str) -> impl Iterator<Item = &str> {
    body.lines()
        .map(|raw| raw.split('#').next().unwrap_or("").trim())
        .filter(|s| !s.is_empty())
}

pub fn sanitize_tag(s: &str) -> String {
    s.chars()
        .map(|c| {
            let lc = c.to_ascii_lowercase();
            if lc.is_ascii_alphanumeric() || matches!(lc, '_' | '.' | '-') {
                lc
            } else {
                '-'
            }
        })
        .collect()
}

pub fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    if s == "~" {
        return home_dir();
    }
    PathBuf::from(s)
}

pub fn canonical(dir: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(dir).ok()
}

pub fn confirm(prompt: &str) -> bool {
    eprint!("sbx: {prompt} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut buf = String::new();
    let tty = std::fs::OpenOptions::new().read(true).open("/dev/tty");
    let result = match tty {
        Ok(mut f) => {
            use std::io::Read;
            let mut byte = [0u8; 1];
            while let Ok(1) = f.read(&mut byte) {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0] as char);
            }
            buf
        }
        Err(_) => return false,
    };
    matches!(result.trim(), "y" | "Y" | "yes" | "YES")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_lines_strips_comments_and_blanks() {
        let body = "# header\n\nfoo\nbar # trailing\n  \nbaz#nospace\n";
        let lines: Vec<&str> = config_lines(body).collect();
        assert_eq!(lines, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn config_lines_empty_input_yields_no_lines() {
        let lines: Vec<&str> = config_lines("").collect();
        assert!(lines.is_empty());
    }

    #[test]
    fn sanitize_tag_lowercases_and_keeps_allowed_chars() {
        assert_eq!(sanitize_tag("My-Tag_1.2"), "my-tag_1.2");
    }

    #[test]
    fn sanitize_tag_replaces_disallowed_chars_with_dash() {
        assert_eq!(sanitize_tag("a/b c@d"), "a-b-c-d");
    }

    #[test]
    fn sanitize_tag_handles_empty() {
        assert_eq!(sanitize_tag(""), "");
    }

    #[test]
    fn expand_tilde_resolves_home_prefix() {
        let cfg = std::env::temp_dir().join("sbx-util-tilde-cfg");
        let home = std::env::temp_dir().join("sbx-util-tilde-home");
        let _ = std::fs::create_dir_all(&home);
        let home_clone = home.clone();
        let _g = set_test_paths(cfg, home);
        assert_eq!(expand_tilde("~/.config"), home_clone.join(".config"));
    }

    #[test]
    fn expand_tilde_lone_tilde_is_home() {
        let cfg = std::env::temp_dir().join("sbx-util-tilde2-cfg");
        let home = std::env::temp_dir().join("sbx-util-tilde2-home");
        let _ = std::fs::create_dir_all(&home);
        let home_clone = home.clone();
        let _g = set_test_paths(cfg, home);
        assert_eq!(expand_tilde("~"), home_clone);
    }

    #[test]
    fn expand_tilde_passes_through_absolute_path() {
        assert_eq!(expand_tilde("/etc/hosts"), PathBuf::from("/etc/hosts"));
    }

    #[test]
    fn expand_tilde_does_not_expand_unanchored_tilde() {
        assert_eq!(expand_tilde("~user/foo"), PathBuf::from("~user/foo"));
    }
}
