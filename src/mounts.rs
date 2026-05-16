use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::project::sbx_file;
use crate::util::{config_dir, expand_tilde, log};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub host: PathBuf,
    pub container: PathBuf,
    pub ro: bool,
}

pub fn resolve(cwd: &Path, container_home: &Path, cli: &[String]) -> Vec<Mount> {
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    let mut out: Vec<Mount> = Vec::new();
    let cwd_canon = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());

    let mut push = |raw: &str| {
        let Some(parsed) = parse_line(raw, container_home) else {
            return;
        };
        if !parsed.host.exists() {
            log(format!(
                "skipping mount (host path does not exist): {}",
                parsed.host.display()
            ));
            return;
        }
        let canon_host = std::fs::canonicalize(&parsed.host).unwrap_or(parsed.host.clone());
        if canon_host == cwd_canon {
            return;
        }
        if seen.insert(canon_host.clone()) {
            out.push(Mount {
                host: canon_host,
                container: parsed.container,
                ro: parsed.ro,
            });
        }
    };

    let global = config_dir().join("mounts");
    if let Ok(contents) = std::fs::read_to_string(&global) {
        for line in contents.lines() {
            push(line);
        }
    }
    let project = sbx_file(cwd, "mounts");
    if let Ok(contents) = std::fs::read_to_string(&project) {
        for line in contents.lines() {
            push(line);
        }
    }
    for raw in cli {
        push(raw);
    }
    out
}

struct ParsedLine {
    host: PathBuf,
    container: PathBuf,
    ro: bool,
}

fn parse_line(raw: &str, container_home: &Path) -> Option<ParsedLine> {
    let trimmed = raw.split('#').next().unwrap_or("").trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<&str> = trimmed.split(':').collect();
    let (host_raw, container_raw, ro) = match parts.as_slice() {
        [h] => (*h, *h, false),
        [h, c] if *c == "ro" => (*h, *h, true),
        [h, c] => (*h, *c, false),
        [h, c, flag] if *flag == "ro" => (*h, *c, true),
        _ => {
            log(format!("ignoring malformed mount line: {raw}"));
            return None;
        }
    };
    let host = expand_tilde(host_raw);
    let container = expand_tilde_with(container_raw, container_home);
    Some(ParsedLine {
        host,
        container,
        ro,
    })
}

fn expand_tilde_with(s: &str, home: &Path) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        return home.join(rest);
    }
    if s == "~" {
        return home.to_path_buf();
    }
    let p = PathBuf::from(s);
    if p.is_absolute() {
        return p;
    }
    expand_tilde(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::home_dir;

    fn home() -> PathBuf {
        home_dir()
    }

    #[test]
    fn bare_path_same_on_both_sides() {
        let p = parse_line("/etc/foo", Path::new("/home/dev")).unwrap();
        assert_eq!(p.host, PathBuf::from("/etc/foo"));
        assert_eq!(p.container, PathBuf::from("/etc/foo"));
        assert!(!p.ro);
    }

    #[test]
    fn tilde_translates_per_side() {
        let p = parse_line("~/.m2", Path::new("/home/dev")).unwrap();
        assert_eq!(p.host, home().join(".m2"));
        assert_eq!(p.container, PathBuf::from("/home/dev/.m2"));
    }

    #[test]
    fn explicit_container_uses_container_home() {
        let p = parse_line(
            "~/.m2/settings.xml:~/.m2/settings.xml",
            Path::new("/home/dev"),
        )
        .unwrap();
        assert_eq!(p.host, home().join(".m2/settings.xml"));
        assert_eq!(p.container, PathBuf::from("/home/dev/.m2/settings.xml"));
    }

    #[test]
    fn ro_suffix() {
        let p = parse_line("/a:/b:ro", Path::new("/home/dev")).unwrap();
        assert_eq!(p.host, PathBuf::from("/a"));
        assert_eq!(p.container, PathBuf::from("/b"));
        assert!(p.ro);
    }

    #[test]
    fn ro_suffix_same_path() {
        let p = parse_line("/etc/foo:ro", Path::new("/home/dev")).unwrap();
        assert_eq!(p.host, PathBuf::from("/etc/foo"));
        assert_eq!(p.container, PathBuf::from("/etc/foo"));
        assert!(p.ro);
    }

    #[test]
    fn comment_stripped() {
        assert!(parse_line("# whole line", Path::new("/home/dev")).is_none());
        let p = parse_line("/a:/b   # trailing", Path::new("/home/dev")).unwrap();
        assert_eq!(p.host, PathBuf::from("/a"));
        assert_eq!(p.container, PathBuf::from("/b"));
    }

    #[test]
    fn blank_skipped() {
        assert!(parse_line("", Path::new("/home/dev")).is_none());
        assert!(parse_line("   ", Path::new("/home/dev")).is_none());
    }
}
