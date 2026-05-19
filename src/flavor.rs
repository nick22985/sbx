use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use std::collections::HashMap;

use crate::config::{Config, FlavorConfig, GlobalConfig};
use crate::docker;
use crate::project::{project_base_name, sbx_file};
use crate::util::{config_dir, die, flavors_dir, home_dir, log};

pub fn flavor_dir(flavor: &str) -> PathBuf {
    flavors_dir().join(flavor)
}

pub const INTERNAL_FLAVORS: &[&str] = &["base", "claude"];

pub const BASE_FLAVOR: &str = "base";

fn flavor_depends_on_base(flavor: &str) -> bool {
    flavor != BASE_FLAVOR && is_flavor(BASE_FLAVOR)
}

pub fn is_internal_flavor(name: &str) -> bool {
    INTERNAL_FLAVORS.contains(&name)
}

pub fn is_flavor(name: &str) -> bool {
    flavor_dir(name).join("Dockerfile").is_file()
}

pub fn list_all_flavors() -> Vec<String> {
    let dir = flavors_dir();
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(&dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        if entry.path().join("Dockerfile").is_file()
            && let Some(name) = entry.file_name().to_str()
        {
            out.push(name.to_string());
        }
    }
    out.sort();
    out
}

pub fn list_flavors() -> Vec<String> {
    list_all_flavors()
        .into_iter()
        .filter(|f| !is_internal_flavor(f))
        .collect()
}

pub fn image_name(flavor: &str) -> String {
    format!("sbx-{flavor}:latest")
}

pub fn project_image_tag(flavor: &str, project_root: &Path) -> String {
    format!("sbx-{}-{}:latest", flavor, project_base_name(project_root))
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CacheEntry {
    HostBind { host_rel: String, container: String },
    Volume { name: String, container: String },
}

impl CacheEntry {
    fn container(&self) -> &str {
        match self {
            CacheEntry::HostBind { container, .. } => container,
            CacheEntry::Volume { container, .. } => container,
        }
    }
}

fn parse_cache_lines<S: AsRef<str>>(lines: &[S], container_home: &Path) -> Vec<CacheEntry> {
    let resolve = |raw: &str| -> String {
        if raw.starts_with('/') {
            raw.to_string()
        } else {
            container_home.join(raw).display().to_string()
        }
    };
    let mut out = Vec::new();
    for raw in lines {
        let line = raw.as_ref().split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('@') {
            let Some((name, container)) = rest.split_once(':') else {
                log(format!(
                    "ignoring malformed volume cache line (need @name:/container): {}",
                    raw.as_ref()
                ));
                continue;
            };
            let name = name.trim().to_string();
            let container_raw = container.trim();
            if name.is_empty() || container_raw.is_empty() {
                continue;
            }
            out.push(CacheEntry::Volume {
                name,
                container: resolve(container_raw),
            });
            continue;
        }
        let (host_rel, container) = match line.split_once(':') {
            Some((h, c)) => (h.trim().to_string(), resolve(c.trim())),
            None => (
                line.to_string(),
                container_home.join(line).display().to_string(),
            ),
        };
        if host_rel.is_empty() || container.is_empty() {
            continue;
        }
        out.push(CacheEntry::HostBind {
            host_rel,
            container,
        });
    }
    out
}

fn flavor_cache_entries(flavor: &str, container_home: &Path) -> Vec<CacheEntry> {
    parse_cache_lines(
        &FlavorConfig::load_or_default(flavor).caches,
        container_home,
    )
}

fn user_global_cache_entries(container_home: &Path) -> Vec<CacheEntry> {
    parse_cache_lines(&GlobalConfig::load_or_default().caches, container_home)
}

fn project_cache_entries(project_root: &Path, container_home: &Path) -> Vec<CacheEntry> {
    parse_cache_lines(
        &Config::load_or_default(project_root).caches,
        container_home,
    )
}

fn merge_cache_layers(layers: &[Vec<CacheEntry>]) -> Vec<CacheEntry> {
    let mut out: Vec<CacheEntry> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for layer in layers {
        for entry in layer {
            let key = entry.container().to_string();
            match index.get(&key) {
                Some(&i) => out[i] = entry.clone(),
                None => {
                    index.insert(key, out.len());
                    out.push(entry.clone());
                }
            }
        }
    }
    out
}

fn effective_cache_entries(
    flavor: &str,
    project_root: Option<&Path>,
    container_home: &Path,
) -> Vec<CacheEntry> {
    let mut layers = vec![
        flavor_cache_entries(flavor, container_home),
        user_global_cache_entries(container_home),
    ];
    if let Some(root) = project_root {
        layers.push(project_cache_entries(root, container_home));
    }
    merge_cache_layers(&layers)
}

pub fn flavor_container_home(_flavor: &str) -> PathBuf {
    container_home()
}

pub fn container_home() -> PathBuf {
    match GlobalConfig::load_or_default().container_home {
        Some(s) if !s.trim().is_empty() => PathBuf::from(s.trim()),
        _ => home_dir(),
    }
}

pub fn cache_args(flavor: &str, project_root: Option<&Path>, container_home: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in effective_cache_entries(flavor, project_root, container_home) {
        match entry {
            CacheEntry::HostBind {
                host_rel,
                container,
            } => {
                let host = home_dir().join(&host_rel);
                let _ = fs::create_dir_all(&host);
                out.push("-v".into());
                out.push(format!("{}:{container}", host.display()));
            }
            CacheEntry::Volume { name, container } => {
                out.push("-v".into());
                out.push(format!("{name}:{container}"));
            }
        }
    }
    out
}

pub fn flavor_volumes(flavor: &str) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let container_home = flavor_container_home(flavor);
    for entry in flavor_cache_entries(flavor, &container_home) {
        if let CacheEntry::Volume { name, .. } = entry {
            v.push(name);
        }
    }
    v
}

pub fn image_exists_or_build(flavor: &str) {
    if flavor_depends_on_base(flavor) && !docker::image_exists(&image_name(BASE_FLAVOR)) {
        build_image(BASE_FLAVOR, false);
    }
    if !docker::image_exists(&image_name(flavor)) {
        build_image(flavor, false);
    }
}

pub fn flavor_context_max_mtime(flavor: &str) -> u64 {
    fn walk(dir: &Path, max: &mut u64) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                walk(&entry.path(), max);
            } else if let Ok(mt) = meta.modified()
                && let Ok(d) = mt.duration_since(std::time::UNIX_EPOCH)
            {
                let s = d.as_secs();
                if s > *max {
                    *max = s;
                }
            }
        }
    }
    let mut max = 0u64;
    walk(&flavor_dir(flavor), &mut max);
    max
}

fn build_stamps_dir() -> PathBuf {
    config_dir().join("build-stamps")
}

fn sanitize_stamp_key(key: &str) -> String {
    key.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '_',
        })
        .collect()
}

fn build_stamp_path(stamp_key: &str) -> PathBuf {
    build_stamps_dir().join(sanitize_stamp_key(stamp_key))
}

fn write_build_stamp(stamp_key: &str) {
    let path = build_stamp_path(stamp_key);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = fs::write(&path, b"") {
        log(format!(
            "failed to write build stamp {}: {e}",
            path.display()
        ));
    }
}

fn read_build_stamp_secs(stamp_key: &str) -> Option<u64> {
    mtime_secs(&build_stamp_path(stamp_key))
}

pub fn image_up_to_date(flavor: &str) -> bool {
    let img = image_name(flavor);
    if !docker::image_exists(&img) {
        return false;
    }
    let img_mt = match read_build_stamp_secs(flavor) {
        Some(t) => t,
        None => return false,
    };
    let mut max_ctx = flavor_context_max_mtime(flavor);
    if flavor_depends_on_base(flavor) {
        let base_mt = flavor_context_max_mtime(BASE_FLAVOR);
        if base_mt > max_ctx {
            max_ctx = base_mt;
        }
    }
    img_mt >= max_ctx
}

fn project_image_up_to_date(
    flavor: &str,
    _project_root: &Path,
    project_df: &Path,
    img: &str,
) -> bool {
    if !docker::image_exists(img) {
        return false;
    }
    let img_mt = match read_build_stamp_secs(img) {
        Some(t) => t,
        None => return false,
    };
    let mut max_ctx = mtime_secs(project_df).unwrap_or(0);
    let flavor_mt = flavor_context_max_mtime(flavor);
    if flavor_mt > max_ctx {
        max_ctx = flavor_mt;
    }
    if flavor_depends_on_base(flavor) {
        let base_mt = flavor_context_max_mtime(BASE_FLAVOR);
        if base_mt > max_ctx {
            max_ctx = base_mt;
        }
    }
    img_mt >= max_ctx
}

fn build_cmd(flavor: &str, no_cache: bool) -> Result<(Command, String, PathBuf), String> {
    if !is_flavor(flavor) {
        return Err(format!(
            "unknown flavor: {flavor} (have: {})",
            list_flavors().join(",")
        ));
    }
    let ctx = flavor_dir(flavor);
    let tag = image_name(flavor);
    let uid = nix_uid();
    let gid = nix_gid();
    let user_home = flavor_container_home(flavor).display().to_string();
    let mut cmd = Command::new("docker");
    cmd.args(["buildx", "build", "--load"]);
    let builder = std::env::var("SBX_BUILDX_BUILDER").unwrap_or_else(|_| "default".into());
    if !builder.is_empty() {
        cmd.args(["--builder", &builder]);
    }
    if no_cache {
        cmd.arg("--no-cache");
    }
    cmd.args([
        "--build-arg",
        &format!("USER_UID={uid}"),
        "--build-arg",
        &format!("USER_GID={gid}"),
        "--build-arg",
        &format!("USER_HOME={user_home}"),
        "-t",
        &tag,
    ])
    .arg(&ctx);
    Ok((cmd, tag, ctx))
}

pub fn build_image(flavor: &str, no_cache: bool) {
    if flavor_depends_on_base(flavor) && !docker::image_exists(&image_name(BASE_FLAVOR)) {
        build_image(BASE_FLAVOR, no_cache);
    }
    let (mut cmd, tag, ctx) = match build_cmd(flavor, no_cache) {
        Ok(v) => v,
        Err(e) => die(e),
    };
    log(format!("building {tag} from {}", ctx.display()));
    let status = cmd.status().unwrap_or_else(|e| die(format!("docker: {e}")));
    if !status.success() {
        die("docker build failed");
    }
    write_build_stamp(flavor);
}

pub fn build_image_streamed(
    flavor: &str,
    no_cache: bool,
    prefix: &str,
    out_lock: Arc<Mutex<()>>,
) -> Result<(), String> {
    let (mut cmd, tag, ctx) = build_cmd(flavor, no_cache)?;
    {
        let _g = out_lock.lock().unwrap();
        eprintln!("[{prefix}] building {tag} from {}", ctx.display());
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("docker: {e}"))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let p1 = prefix.to_string();
    let l1 = out_lock.clone();
    let t1 = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _g = l1.lock().unwrap();
            eprintln!("[{p1}] {line}");
        }
    });
    let p2 = prefix.to_string();
    let l2 = out_lock.clone();
    let t2 = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            let _g = l2.lock().unwrap();
            eprintln!("[{p2}] {line}");
        }
    });
    let status = child.wait().map_err(|e| format!("docker: {e}"))?;
    t1.join().ok();
    t2.join().ok();
    if !status.success() {
        return Err(format!("docker build failed for {flavor}"));
    }
    write_build_stamp(flavor);
    Ok(())
}

pub fn resolve_image(flavor: &str, project_root: &Path, no_cache: bool) -> String {
    if flavor_depends_on_base(flavor) && !image_up_to_date(BASE_FLAVOR) {
        build_image(BASE_FLAVOR, false);
    }
    let base = image_name(flavor);
    if !image_up_to_date(flavor) {
        build_image(flavor, false);
    }

    let project_df = sbx_file(project_root, "Dockerfile");
    if !project_df.is_file() {
        return base;
    }

    let img = project_image_tag(flavor, project_root);
    let needs_build = !project_image_up_to_date(flavor, project_root, &project_df, &img);
    if no_cache || needs_build {
        log(format!(
            "building project image {img} from {}",
            project_df.display()
        ));
        let uid = nix_uid();
        let gid = nix_gid();
        let user_home = home_dir().display().to_string();
        let mut cmd = Command::new("docker");
        cmd.args(["buildx", "build", "--load"]);
        let builder = std::env::var("SBX_BUILDX_BUILDER").unwrap_or_else(|_| "default".into());
        if !builder.is_empty() {
            cmd.args(["--builder", &builder]);
        }
        if no_cache {
            cmd.arg("--no-cache");
        }
        cmd.args([
            "--build-arg",
            &format!("USER_UID={uid}"),
            "--build-arg",
            &format!("USER_GID={gid}"),
            "--build-arg",
            &format!("USER_HOME={user_home}"),
            "-t",
            &img,
            "-f",
        ])
        .arg(&project_df);
        cmd.arg(project_df.parent().unwrap_or(project_root));
        let status = cmd.status().unwrap_or_else(|e| die(format!("docker: {e}")));
        if !status.success() {
            die("docker build failed");
        }
        write_build_stamp(&img);
    }
    img
}

fn mtime_secs(p: &Path) -> Option<u64> {
    let meta = fs::metadata(p).ok()?;
    let mt = meta.modified().ok()?;
    mt.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

pub fn nix_uid() -> u32 {
    unsafe { libc_compat::getuid() }
}

pub fn nix_gid() -> u32 {
    unsafe { libc_compat::getgid() }
}

mod libc_compat {
    unsafe extern "C" {
        pub fn getuid() -> u32;
        pub fn getgid() -> u32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev_home() -> PathBuf {
        PathBuf::from("/home/dev")
    }

    #[test]
    fn parses_bare_host_rel_defaults_container_under_home_dev() {
        let entries = parse_cache_lines(&[".npm"], &dev_home());
        assert_eq!(
            entries,
            vec![CacheEntry::HostBind {
                host_rel: ".npm".into(),
                container: "/home/dev/.npm".into(),
            }]
        );
    }

    #[test]
    fn parses_explicit_container_path() {
        let entries = parse_cache_lines(&[".m2:/home/dev/.m2"], &dev_home());
        assert_eq!(
            entries,
            vec![CacheEntry::HostBind {
                host_rel: ".m2".into(),
                container: "/home/dev/.m2".into(),
            }]
        );
    }

    #[test]
    fn parses_named_volume() {
        let entries = parse_cache_lines(&["@sbx-maven-cache:/home/dev/.m2"], &dev_home());
        assert_eq!(
            entries,
            vec![CacheEntry::Volume {
                name: "sbx-maven-cache".into(),
                container: "/home/dev/.m2".into(),
            }]
        );
    }

    #[test]
    fn strips_comments_and_blanks() {
        let entries = parse_cache_lines(&["# comment", "", ".npm  # trailing"], &dev_home());
        assert_eq!(entries.len(), 1);
        assert!(matches!(&entries[0], CacheEntry::HostBind { host_rel, .. } if host_rel == ".npm"));
    }

    #[test]
    fn malformed_volume_line_skipped() {
        let entries = parse_cache_lines(&["@bad-no-colon", ".npm"], &dev_home());
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn user_entry_overrides_flavor_on_same_container_path() {
        let flavor = parse_cache_lines(&["@sbx-maven-cache:/home/dev/.m2"], &dev_home());
        let user = parse_cache_lines(&["@sbx-maven-mine:/home/dev/.m2"], &dev_home());
        let merged = merge_cache_layers(&[flavor, user]);
        assert_eq!(
            merged,
            vec![CacheEntry::Volume {
                name: "sbx-maven-mine".into(),
                container: "/home/dev/.m2".into(),
            }]
        );
    }

    #[test]
    fn host_bind_overrides_volume_on_same_container_path() {
        let flavor = parse_cache_lines(&["@sbx-maven-cache:/home/dev/.m2"], &dev_home());
        let user = parse_cache_lines(&[".m2:/home/dev/.m2"], &dev_home());
        let merged = merge_cache_layers(&[flavor, user]);
        assert_eq!(
            merged,
            vec![CacheEntry::HostBind {
                host_rel: ".m2".into(),
                container: "/home/dev/.m2".into(),
            }]
        );
    }

    #[test]
    fn distinct_container_paths_layer_additively() {
        let flavor = parse_cache_lines(&[".npm"], &dev_home());
        let global = parse_cache_lines(&[".cargo/registry"], &dev_home());
        let project = parse_cache_lines(&[".gradle"], &dev_home());
        let merged = merge_cache_layers(&[flavor, global, project]);
        assert_eq!(merged.len(), 3);
        let containers: Vec<&str> = merged.iter().map(|e| e.container()).collect();
        assert_eq!(
            containers,
            vec![
                "/home/dev/.npm",
                "/home/dev/.cargo/registry",
                "/home/dev/.gradle",
            ]
        );
    }

    #[test]
    fn bare_host_rel_resolves_under_host_home_when_set() {
        let host_home = PathBuf::from("/home/nick");
        let entries = parse_cache_lines(&[".local/share/nvim"], &host_home);
        assert_eq!(
            entries,
            vec![CacheEntry::HostBind {
                host_rel: ".local/share/nvim".into(),
                container: "/home/nick/.local/share/nvim".into(),
            }]
        );
    }

    #[test]
    fn volume_relative_container_path_resolves_under_container_home() {
        let entries = parse_cache_lines(
            &["@sbx-claude-local:.local"],
            &PathBuf::from("/home/nick"),
        );
        assert_eq!(
            entries,
            vec![CacheEntry::Volume {
                name: "sbx-claude-local".into(),
                container: "/home/nick/.local".into(),
            }]
        );
    }

    #[test]
    fn volume_absolute_container_path_passes_through() {
        let entries = parse_cache_lines(&["@vol:/opt/data"], &PathBuf::from("/home/nick"));
        assert_eq!(
            entries,
            vec![CacheEntry::Volume {
                name: "vol".into(),
                container: "/opt/data".into(),
            }]
        );
    }

    #[test]
    fn host_bind_relative_container_path_resolves_under_container_home() {
        let entries = parse_cache_lines(
            &[".local/share/mise:.local/share/mise"],
            &PathBuf::from("/home/nick"),
        );
        assert_eq!(
            entries,
            vec![CacheEntry::HostBind {
                host_rel: ".local/share/mise".into(),
                container: "/home/nick/.local/share/mise".into(),
            }]
        );
    }

    #[test]
    fn project_layer_overrides_global_layer() {
        let flavor: Vec<CacheEntry> = parse_cache_lines::<&str>(&[], &dev_home());
        let global = parse_cache_lines(&["@globalvol:/home/dev/x"], &dev_home());
        let project = parse_cache_lines(&["@projectvol:/home/dev/x"], &dev_home());
        let merged = merge_cache_layers(&[flavor, global, project]);
        assert_eq!(
            merged,
            vec![CacheEntry::Volume {
                name: "projectvol".into(),
                container: "/home/dev/x".into(),
            }]
        );
    }

    use crate::util::set_test_paths;

    fn tmp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("sbx-flavor-{label}-{pid}-{nanos}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn container_home_defaults_to_host_home() {
        let cfg = tmp_dir("ch-default-cfg");
        let home = tmp_dir("ch-default-home");
        let home_clone = home.clone();
        let _g = set_test_paths(cfg, home);
        assert_eq!(container_home(), home_clone);
        assert_eq!(flavor_container_home("rust"), home_clone);
    }

    #[test]
    fn container_home_uses_global_config_override() {
        let cfg = tmp_dir("ch-override-cfg");
        let home = tmp_dir("ch-override-home");
        let _g = set_test_paths(cfg, home);
        GlobalConfig {
            container_home: Some("/opt/dev".into()),
            ..Default::default()
        }
        .save()
        .unwrap();
        assert_eq!(container_home(), PathBuf::from("/opt/dev"));
        assert_eq!(flavor_container_home("rust"), PathBuf::from("/opt/dev"));
    }

    #[test]
    fn container_home_treats_blank_override_as_unset() {
        let cfg = tmp_dir("ch-blank-cfg");
        let home = tmp_dir("ch-blank-home");
        let home_clone = home.clone();
        let _g = set_test_paths(cfg, home);
        GlobalConfig {
            container_home: Some("   ".into()),
            ..Default::default()
        }
        .save()
        .unwrap();
        assert_eq!(container_home(), home_clone);
    }

    #[test]
    fn cache_args_emits_only_user_declared_entries() {
        let cfg = tmp_dir("ca-mise-cfg");
        let home = tmp_dir("ca-mise-home");
        let _g = set_test_paths(cfg, home);
        // No flavor / global / project config written, so caches are empty.
        let args = cache_args("rust", None, &PathBuf::from("/home/nick"));
        assert!(args.is_empty(), "expected no hardcoded mounts, got {args:?}");
    }

    #[test]
    fn cache_args_passes_through_flavor_config_volumes() {
        let cfg = tmp_dir("ca-flavor-vols-cfg");
        let home = tmp_dir("ca-flavor-vols-home");
        let _g = set_test_paths(cfg, home);
        FlavorConfig {
            caches: vec![
                "@sbx-mise-rust:.local/share/mise".into(),
                "@sbx-mise-state-rust:.local/state/mise".into(),
            ],
            ..Default::default()
        }
        .save("rust")
        .unwrap();
        let args = cache_args("rust", None, &PathBuf::from("/home/nick"));
        assert!(
            args.windows(2)
                .any(|w| w[0] == "-v" && w[1] == "sbx-mise-rust:/home/nick/.local/share/mise"),
            "missing share mount in {args:?}"
        );
        assert!(
            args.windows(2)
                .any(|w| w[0] == "-v" && w[1] == "sbx-mise-state-rust:/home/nick/.local/state/mise"),
            "missing state mount in {args:?}"
        );
    }

    #[test]
    fn cache_args_no_implicit_claude_local_volume() {
        let cfg = tmp_dir("ca-claude-cfg");
        let home = tmp_dir("ca-claude-home");
        let _g = set_test_paths(cfg, home);
        let args = cache_args("claude", None, &PathBuf::from("/home/dev"));
        assert!(
            !args.iter().any(|a| a.starts_with("sbx-claude-local")),
            "claude flavor should not get an implicit sbx-claude-local mount: {args:?}"
        );
        assert!(args.is_empty(), "expected no hardcoded mounts, got {args:?}");
    }

    #[test]
    fn flavor_context_max_mtime_returns_zero_for_missing_dir() {
        let cfg = tmp_dir("ctx-missing-cfg");
        let home = tmp_dir("ctx-missing-home");
        let _g = set_test_paths(cfg, home);
        assert_eq!(flavor_context_max_mtime("does-not-exist"), 0);
    }

    #[test]
    fn build_stamp_roundtrip_records_mtime() {
        let cfg = tmp_dir("stamp-cfg");
        let home = tmp_dir("stamp-home");
        let _g = set_test_paths(cfg, home);
        assert!(read_build_stamp_secs("rust").is_none());
        write_build_stamp("rust");
        let mt = read_build_stamp_secs("rust").expect("stamp should exist");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(mt <= now && now - mt < 5);
    }

    #[test]
    fn build_stamp_path_sanitizes_unsafe_characters() {
        let cfg = tmp_dir("stamp-sanitize-cfg");
        let home = tmp_dir("stamp-sanitize-home");
        let _g = set_test_paths(cfg, home);
        let p = build_stamp_path("sbx-rust-myapp:latest");
        let name = p.file_name().unwrap().to_string_lossy();
        assert_eq!(name, "sbx-rust-myapp_latest");
    }

    #[test]
    fn flavor_context_max_mtime_picks_max_across_nested_files() {
        let cfg = tmp_dir("ctx-nested-cfg");
        let home = tmp_dir("ctx-nested-home");
        let _g = set_test_paths(cfg.clone(), home);
        let fdir = cfg.join("flavors").join("demo");
        fs::create_dir_all(fdir.join("sub")).unwrap();
        fs::write(fdir.join("Dockerfile"), "FROM x").unwrap();
        fs::write(fdir.join("sub/extra"), "data").unwrap();
        let mt = flavor_context_max_mtime("demo");
        assert!(mt > 0);
        let df_mt = fs::metadata(fdir.join("Dockerfile"))
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let sub_mt = fs::metadata(fdir.join("sub/extra"))
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(mt, df_mt.max(sub_mt));
    }
}
