use std::path::{Path, PathBuf};

use crate::config::{Agent, Config, FlavorConfig};
use crate::util::{config_dir, die, home_dir, log};

fn profiles_root(flavor: &str) -> PathBuf {
    config_dir().join(format!("{flavor}-profiles"))
}

fn profile_dir(flavor: &str, name: &str) -> PathBuf {
    profiles_root(flavor).join(name)
}

fn profile_exists(flavor: &str, name: &str) -> bool {
    profile_dir(flavor, name).is_dir()
}

fn validate_profile_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("profile name cannot be empty".into());
    }
    if name.contains('/') || name.contains("..") || name.starts_with('.') {
        return Err(format!("invalid profile name: {name}"));
    }
    Ok(())
}

pub fn list_profiles(flavor: &str) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(profiles_root(flavor)) else {
        return out;
    };
    for e in entries.flatten() {
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false)
            && let Some(name) = e.file_name().to_str()
        {
            out.push(name.to_string());
        }
    }
    out.sort();
    out
}

pub fn resolve(flavor: &str, cwd: &Path, cli: Option<&str>) -> Option<String> {
    if let Some(raw) = cli {
        let name = raw.trim();
        if name.is_empty() {
            return None;
        }
        if !profile_exists(flavor, name) {
            die(format!(
                "no such {flavor} profile: {name}  (have: {})",
                list_profiles(flavor).join(", ")
            ));
        }
        return Some(name.to_string());
    }
    if let Some(pinned) = Config::load_or_default(cwd).profiles.get(flavor) {
        let name = pinned.trim();
        if !name.is_empty() {
            if !profile_exists(flavor, name) {
                die(format!(
                    ".sbx/config.toml pins {flavor} profile {name}, but it doesn't exist (have: {})",
                    list_profiles(flavor).join(", ")
                ));
            }
            return Some(name.to_string());
        }
    }
    None
}

pub fn mount_args(
    cwd: &Path,
    flavor: &str,
    agent: &Agent,
    cli_profile: Option<&str>,
    container_home: &Path,
) -> Vec<String> {
    let profile = if agent.profiles {
        resolve(flavor, cwd, cli_profile)
    } else {
        None
    };
    if let Some(name) = &profile {
        log(format!("{flavor} profile: {name}"));
    }

    // Container-relative dirs bound from the host regardless of profile. A
    // scoped file nested under one of these is already provided by it when no
    // profile is active, so it's only mounted as an overlay when profiled.
    let shared_dirs: Vec<PathBuf> = agent
        .persist
        .iter()
        .map(|p| p.spec())
        .filter(|s| s.shared && !s.file)
        .map(|s| PathBuf::from(s.path))
        .collect();

    let home = home_dir();
    let cs = container_home.display();
    let mut out = Vec::new();

    for entry in &agent.persist {
        let spec = entry.spec();
        let target = format!("{cs}/{}", spec.path);

        // Pick the host-side source: a shared entry always comes from the host
        // home; a scoped entry comes from the profile dir when one is active,
        // else the host home (unless a shared dir already covers it).
        let src = if spec.shared {
            home.join(&spec.path)
        } else if let Some(name) = &profile {
            profile_dir(flavor, name).join(spec.store_path())
        } else {
            if shared_dirs
                .iter()
                .any(|d| Path::new(&spec.path).starts_with(d))
            {
                continue;
            }
            home.join(&spec.path)
        };

        if spec.file {
            if !src.is_file() {
                if spec.optional {
                    continue;
                }
                if let Some(parent) = src.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&src, spec.seed_default.as_deref().unwrap_or(""));
            }
        } else {
            let _ = std::fs::create_dir_all(&src);
        }

        out.push("-v".into());
        out.push(format!("{}:{target}", src.display()));
    }

    out
}

pub fn add(flavor: &str, name: &str) {
    if let Err(e) = validate_profile_name(name) {
        die(e);
    }
    let dir = profile_dir(flavor, name);
    if dir.exists() {
        die(format!("{flavor} profile already exists: {name}"));
    }
    if let Err(e) = std::fs::create_dir_all(&dir) {
        die(format!("create {}: {e}", dir.display()));
    }
    seed_profile(flavor, &dir);
    log(format!(
        "created {flavor} profile: {name} at {}",
        dir.display()
    ));
    log(format!(
        "next: sbx {flavor} -p {name}   (first run starts logged out, so sign in fresh)"
    ));
}

/// Seed a freshly-created profile dir from the host for any `seed` persist
/// entries, so a new profile inherits config (e.g. MCP servers) but not
/// credentials, and has a file to bind for persistence.
fn seed_profile(flavor: &str, dir: &Path) {
    let Some(agent) = FlavorConfig::load_or_default(flavor).agent else {
        return;
    };
    let home = home_dir();
    for spec in agent.persist.iter().map(|p| p.spec()) {
        if !spec.seed {
            continue;
        }
        let dst = dir.join(spec.store_path());
        if let Some(parent) = dst.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            die(format!("create {}: {e}", parent.display()));
        }
        let src = home.join(&spec.path);
        if src.is_file() {
            if let Err(e) = std::fs::copy(&src, &dst) {
                die(format!("seed {}: {e}", spec.path));
            }
        } else if let Err(e) = std::fs::write(&dst, spec.seed_default.as_deref().unwrap_or("")) {
            die(format!("write {}: {e}", dst.display()));
        }
    }
}

pub fn remove(flavor: &str, name: &str) {
    if let Err(e) = validate_profile_name(name) {
        die(e);
    }
    let dir = profile_dir(flavor, name);
    if !dir.is_dir() {
        die(format!("no such {flavor} profile: {name}"));
    }
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        die(format!("remove {}: {e}", dir.display()));
    }
    log(format!("removed {flavor} profile: {name}"));
}

pub fn print_list(flavor: &str, cwd: &Path) {
    let current = resolve(flavor, cwd, None);
    let profiles = list_profiles(flavor);
    if profiles.is_empty() {
        log(format!(
            "no {flavor} profiles yet - create one with: sbx {flavor} profile add NAME"
        ));
        return;
    }
    for p in profiles {
        let marker = if Some(&p) == current.as_ref() {
            "*"
        } else {
            " "
        };
        println!("{marker} {p}");
    }
}

pub fn print_current(flavor: &str, cwd: &Path, cli_profile: Option<&str>) {
    match resolve(flavor, cwd, cli_profile) {
        Some(name) => println!("{name}"),
        None => println!("(none, using host config)"),
    }
}

pub fn dispatch(flavor: &str, cwd: &Path, args: &[String]) -> bool {
    if args.first().map(String::as_str) != Some("profile") {
        return false;
    }
    let usage = || format!("usage: sbx {flavor} profile [list|add NAME|rm NAME|current]");
    match args.get(1).map(String::as_str).unwrap_or("list") {
        "list" | "ls" => print_list(flavor, cwd),
        "current" => print_current(flavor, cwd, None),
        "add" => match args.get(2) {
            Some(name) => add(flavor, name),
            None => die(format!("usage: sbx {flavor} profile add NAME")),
        },
        "rm" | "remove" | "del" | "delete" => match args.get(2) {
            Some(name) => remove(flavor, name),
            None => die(format!("usage: sbx {flavor} profile rm NAME")),
        },
        other => die(format!("unknown profile subcommand: {other}\n{}", usage())),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Persist, PersistSpec};
    use crate::util::set_test_paths;

    fn tmp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("sbx-prof-{label}-{pid}-{nanos}"));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn opencode_agent() -> Agent {
        Agent {
            persist: vec![
                ".config/opencode".into(),
                ".local/share/opencode".into(),
            ],
            profiles: true,
            ..Default::default()
        }
    }

    // Mirrors the claude flavor's `[agent].persist`: a shared session dir, a
    // seeded per-profile `.claude.json`, and a per-profile credentials file
    // overlaid inside the shared dir.
    fn claude_agent() -> Agent {
        Agent {
            persist: vec![
                Persist::Detailed(PersistSpec {
                    path: ".claude".into(),
                    shared: true,
                    ..Default::default()
                }),
                Persist::Detailed(PersistSpec {
                    path: ".claude.json".into(),
                    file: true,
                    seed: true,
                    seed_default: Some("{}\n".into()),
                    optional: true,
                    ..Default::default()
                }),
                Persist::Detailed(PersistSpec {
                    path: ".claude/.credentials.json".into(),
                    file: true,
                    store: Some(".credentials.json".into()),
                    ..Default::default()
                }),
            ],
            profiles: true,
            ..Default::default()
        }
    }

    #[test]
    fn claude_unprofiled_binds_shared_dir_and_host_json_but_not_credentials() {
        let cfg = tmp_dir("cl-up-cfg");
        let home = tmp_dir("cl-up-home");
        let _g = set_test_paths(cfg, home.clone());
        std::fs::write(home.join(".claude.json"), "{\"host\":true}").unwrap();

        let args = mount_args(
            Path::new("/nope"),
            "claude",
            &claude_agent(),
            None,
            Path::new("/home/dev"),
        );
        // `.claude` from host, `.claude.json` from host; `.credentials.json` is
        // skipped (the shared `.claude` mount already provides it).
        assert_eq!(
            args,
            vec![
                "-v".to_string(),
                format!("{}/.claude:/home/dev/.claude", home.display()),
                "-v".to_string(),
                format!("{}/.claude.json:/home/dev/.claude.json", home.display()),
            ]
        );
    }

    #[test]
    fn claude_unprofiled_skips_absent_optional_host_json() {
        let cfg = tmp_dir("cl-opt-cfg");
        let home = tmp_dir("cl-opt-home");
        let _g = set_test_paths(cfg, home.clone());
        // no host ~/.claude.json — optional entry must not fabricate it

        let args = mount_args(
            Path::new("/nope"),
            "claude",
            &claude_agent(),
            None,
            Path::new("/home/dev"),
        );
        assert_eq!(
            args,
            vec![
                "-v".to_string(),
                format!("{}/.claude:/home/dev/.claude", home.display()),
            ]
        );
        assert!(!home.join(".claude.json").exists());
    }

    #[test]
    fn claude_profiled_overlays_profile_json_and_credentials() {
        let cfg = tmp_dir("cl-p-cfg");
        let home = tmp_dir("cl-p-home");
        let _g = set_test_paths(cfg.clone(), home.clone());
        let pdir = cfg.join("claude-profiles").join("work");
        std::fs::create_dir_all(&pdir).unwrap();
        std::fs::write(pdir.join(".claude.json"), "{}\n").unwrap();

        let args = mount_args(
            Path::new("/nope"),
            "claude",
            &claude_agent(),
            Some("work"),
            Path::new("/home/dev"),
        );
        assert_eq!(
            args,
            vec![
                // `.claude` stays shared with the host even under a profile.
                "-v".to_string(),
                format!("{}/.claude:/home/dev/.claude", home.display()),
                // auth swaps to the profile dir.
                "-v".to_string(),
                format!("{}/.claude.json:/home/dev/.claude.json", pdir.display()),
                "-v".to_string(),
                format!(
                    "{}/.credentials.json:/home/dev/.claude/.credentials.json",
                    pdir.display()
                ),
            ]
        );
        // credentials file is auto-created at its profile-root storage path.
        assert!(pdir.join(".credentials.json").is_file());
    }

    #[test]
    fn add_seeds_from_host_copy_when_present() {
        let cfg = tmp_dir("cl-seed-cfg");
        let home = tmp_dir("cl-seed-home");
        let _g = set_test_paths(cfg.clone(), home.clone());
        let fdir = cfg.join("flavors").join("claudex");
        std::fs::create_dir_all(&fdir).unwrap();
        std::fs::write(
            fdir.join("config.toml"),
            "[agent]\nprofiles = true\npersist = [{ path = \".claude.json\", file = true, seed = true }]\n",
        )
        .unwrap();
        std::fs::write(home.join(".claude.json"), "HOSTDATA").unwrap();

        add("claudex", "work");
        let seeded = cfg
            .join("claudex-profiles")
            .join("work")
            .join(".claude.json");
        assert_eq!(std::fs::read_to_string(seeded).unwrap(), "HOSTDATA");
    }

    #[test]
    fn add_writes_seed_default_when_host_absent() {
        let cfg = tmp_dir("cl-sd-cfg");
        let home = tmp_dir("cl-sd-home");
        let _g = set_test_paths(cfg.clone(), home);
        let fdir = cfg.join("flavors").join("claudey");
        std::fs::create_dir_all(&fdir).unwrap();
        std::fs::write(
            fdir.join("config.toml"),
            "[agent]\nprofiles = true\npersist = [{ path = \".claude.json\", file = true, seed = true, seed-default = \"{}\" }]\n",
        )
        .unwrap();

        add("claudey", "solo");
        let seeded = cfg
            .join("claudey-profiles")
            .join("solo")
            .join(".claude.json");
        assert_eq!(std::fs::read_to_string(seeded).unwrap(), "{}");
    }

    #[test]
    fn add_list_remove_round_trip() {
        let cfg = tmp_dir("alr-cfg");
        let home = tmp_dir("alr-home");
        let _g = set_test_paths(cfg, home);

        assert!(list_profiles("opencode").is_empty());
        add("opencode", "work");
        add("opencode", "personal");
        assert_eq!(list_profiles("opencode"), vec!["personal", "work"]);
        remove("opencode", "work");
        assert_eq!(list_profiles("opencode"), vec!["personal"]);
    }

    #[test]
    fn no_profile_binds_from_host_home() {
        let cfg = tmp_dir("hp-cfg");
        let home = tmp_dir("hp-home");
        let _g = set_test_paths(cfg, home.clone());

        let agent = opencode_agent();
        let args = mount_args(
            Path::new("/nope"),
            "opencode",
            &agent,
            None,
            Path::new("/home/dev"),
        );
        assert_eq!(
            args,
            vec![
                "-v".to_string(),
                format!(
                    "{}/.config/opencode:/home/dev/.config/opencode",
                    home.display()
                ),
                "-v".to_string(),
                format!(
                    "{}/.local/share/opencode:/home/dev/.local/share/opencode",
                    home.display()
                ),
            ]
        );
    }

    #[test]
    fn active_profile_binds_from_profile_dir() {
        let cfg = tmp_dir("ap-cfg");
        let home = tmp_dir("ap-home");
        let _g = set_test_paths(cfg.clone(), home);

        add("opencode", "work");
        let agent = opencode_agent();
        let args = mount_args(
            Path::new("/nope"),
            "opencode",
            &agent,
            Some("work"),
            Path::new("/home/dev"),
        );
        let base = cfg.join("opencode-profiles").join("work");
        assert_eq!(
            args,
            vec![
                "-v".to_string(),
                format!(
                    "{}/.config/opencode:/home/dev/.config/opencode",
                    base.display()
                ),
                "-v".to_string(),
                format!(
                    "{}/.local/share/opencode:/home/dev/.local/share/opencode",
                    base.display()
                ),
            ]
        );
        assert!(base.join(".config/opencode").is_dir());
    }

    #[test]
    fn resolve_reads_project_pin() {
        let cfg = tmp_dir("pin-cfg");
        let home = tmp_dir("pin-home");
        let _g = set_test_paths(cfg, home);
        add("opencode", "work");

        let proj = tmp_dir("pin-proj");
        let mut profiles = std::collections::BTreeMap::new();
        profiles.insert("opencode".to_string(), "work".to_string());
        Config {
            profiles,
            ..Default::default()
        }
        .save_to_dir(&proj.join(".sbx"))
        .unwrap();

        assert_eq!(resolve("opencode", &proj, None).as_deref(), Some("work"));
        add("opencode", "other");
        assert_eq!(
            resolve("opencode", &proj, Some("other")).as_deref(),
            Some("other")
        );
    }

    #[test]
    fn dispatch_only_claims_profile_verb() {
        let cfg = tmp_dir("disp-cfg");
        let home = tmp_dir("disp-home");
        let _g = set_test_paths(cfg, home);
        assert!(!dispatch("opencode", Path::new("/nope"), &["auth".into()]));
        assert!(!dispatch("opencode", Path::new("/nope"), &[]));
        assert!(dispatch(
            "opencode",
            Path::new("/nope"),
            &["profile".into(), "list".into()]
        ));
    }
}
