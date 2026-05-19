use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::config::{Config, FlavorConfig, GlobalConfig};
use crate::util::{expand_tilde, log};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub host: PathBuf,
    pub container: PathBuf,
    pub ro: bool,
}

pub fn resolve(
    cwd: &Path,
    container_home: &Path,
    cli: &[String],
    flavor: Option<&str>,
) -> Vec<Mount> {
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

    if let Some(name) = flavor {
        for raw in FlavorConfig::load_or_default(name).mounts {
            push(&raw);
        }
    }
    for raw in GlobalConfig::load_or_default().mounts {
        push(&raw);
    }
    for raw in Config::load_or_default(cwd).mounts {
        push(&raw);
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
    use crate::util::{home_dir, set_test_paths};

    fn home() -> PathBuf {
        home_dir()
    }

    fn tmp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("sbx-mt-{label}-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn touch_dir(p: &Path) {
        std::fs::create_dir_all(p).unwrap();
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

    #[test]
    fn resolve_layers_flavor_global_project_cli_in_order() {
        let cfg = tmp_dir("layer-cfg");
        let home = tmp_dir("layer-home");
        touch_dir(&home.join("flavor-only"));
        touch_dir(&home.join("global-only"));
        touch_dir(&home.join("project-only"));
        touch_dir(&home.join("cli-only"));
        let _g = set_test_paths(cfg.clone(), home.clone());

        FlavorConfig {
            mounts: vec!["~/flavor-only".into()],
            caches: vec![],
            start: None,
        }
        .save("npm")
        .unwrap();

        GlobalConfig {
            mounts: vec!["~/global-only".into()],
            caches: vec![],
            ..Default::default()
        }
        .save()
        .unwrap();

        let project_root = tmp_dir("layer-proj");
        Config {
            mounts: vec!["~/project-only".into()],
            ..Default::default()
        }
        .save_to_dir(&project_root.join(".sbx"))
        .unwrap();

        let cli = vec!["~/cli-only".to_string()];
        let mounts = resolve(&project_root, Path::new("/home/dev"), &cli, Some("npm"));
        let hosts: Vec<_> = mounts
            .iter()
            .map(|m| m.host.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            hosts,
            vec!["flavor-only", "global-only", "project-only", "cli-only"]
        );
    }

    #[test]
    fn resolve_first_wins_when_layers_collide() {
        let cfg = tmp_dir("dup-cfg");
        let home = tmp_dir("dup-home");
        touch_dir(&home.join("shared"));
        let _g = set_test_paths(cfg.clone(), home.clone());

        FlavorConfig {
            mounts: vec!["~/shared:/flavor-target".into()],
            caches: vec![],
            start: None,
        }
        .save("npm")
        .unwrap();
        GlobalConfig {
            mounts: vec!["~/shared:/global-target".into()],
            caches: vec![],
            ..Default::default()
        }
        .save()
        .unwrap();

        let project_root = tmp_dir("dup-proj");
        let mounts = resolve(&project_root, Path::new("/home/dev"), &[], Some("npm"));
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].container, PathBuf::from("/flavor-target"));
    }

    #[test]
    fn resolve_skips_missing_host_paths() {
        let cfg = tmp_dir("miss-cfg");
        let home = tmp_dir("miss-home");
        touch_dir(&home.join("exists"));
        let _g = set_test_paths(cfg.clone(), home.clone());

        GlobalConfig {
            mounts: vec!["~/exists".into(), "~/does-not-exist".into()],
            caches: vec![],
            ..Default::default()
        }
        .save()
        .unwrap();

        let project_root = tmp_dir("miss-proj");
        let mounts = resolve(&project_root, Path::new("/home/dev"), &[], None);
        assert_eq!(mounts.len(), 1);
        assert!(mounts[0].host.ends_with("exists"));
    }

    #[test]
    fn resolve_skips_cwd_mount() {
        let cfg = tmp_dir("cwd-cfg");
        let home = tmp_dir("cwd-home");
        let _g = set_test_paths(cfg.clone(), home.clone());

        let project_root = tmp_dir("cwd-proj");
        GlobalConfig {
            mounts: vec![project_root.to_string_lossy().to_string()],
            caches: vec![],
            ..Default::default()
        }
        .save()
        .unwrap();

        let mounts = resolve(&project_root, Path::new("/home/dev"), &[], None);
        assert!(
            mounts.is_empty(),
            "cwd should be skipped, got {:?}",
            mounts
        );
    }

    #[test]
    fn resolve_flavor_none_skips_flavor_layer() {
        let cfg = tmp_dir("noflav-cfg");
        let home = tmp_dir("noflav-home");
        touch_dir(&home.join("flav-only"));
        touch_dir(&home.join("glob-only"));
        let _g = set_test_paths(cfg.clone(), home.clone());

        FlavorConfig {
            mounts: vec!["~/flav-only".into()],
            caches: vec![],
            start: None,
        }
        .save("nvim")
        .unwrap();
        GlobalConfig {
            mounts: vec!["~/glob-only".into()],
            caches: vec![],
            ..Default::default()
        }
        .save()
        .unwrap();

        let project_root = tmp_dir("noflav-proj");
        let mounts = resolve(&project_root, Path::new("/home/dev"), &[], None);
        let hosts: Vec<_> = mounts
            .iter()
            .map(|m| m.host.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(hosts, vec!["glob-only"]);
    }

    #[test]
    fn expand_tilde_with_uses_supplied_home_for_prefix() {
        let custom = Path::new("/var/alt");
        assert_eq!(
            expand_tilde_with("~/foo", custom),
            PathBuf::from("/var/alt/foo")
        );
        assert_eq!(expand_tilde_with("~", custom), PathBuf::from("/var/alt"));
    }

    #[test]
    fn expand_tilde_with_passes_absolute_through() {
        assert_eq!(
            expand_tilde_with("/etc/hosts", Path::new("/var/alt")),
            PathBuf::from("/etc/hosts")
        );
    }

    #[test]
    fn expand_tilde_with_falls_back_to_global_for_relative() {
        let cfg = tmp_dir("etw-fallback-cfg");
        let home = tmp_dir("etw-fallback-home");
        let _g = set_test_paths(cfg, home);
        assert_eq!(
            expand_tilde_with("foo/bar", Path::new("/var/alt")),
            PathBuf::from("foo/bar")
        );
    }
}
