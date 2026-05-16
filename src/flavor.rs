use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::docker;
use crate::project::{project_base_name, sbx_file};
use crate::util::{config_dir, die, home_dir, log};

pub fn flavor_dir(flavor: &str) -> PathBuf {
    config_dir().join(flavor)
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
    let dir = config_dir();
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

fn cache_entries(flavor: &str) -> Vec<CacheEntry> {
    let Ok(body) = fs::read_to_string(flavor_dir(flavor).join("caches")) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for raw in body.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix('@') {
            let Some((name, container)) = rest.split_once(':') else {
                log(format!(
                    "ignoring malformed volume cache line (need @name:/container): {raw}"
                ));
                continue;
            };
            let name = name.trim().to_string();
            let container = container.trim().to_string();
            if name.is_empty() || container.is_empty() {
                continue;
            }
            out.push(CacheEntry::Volume { name, container });
            continue;
        }
        let (host_rel, container) = match line.split_once(':') {
            Some((h, c)) => (h.trim().to_string(), c.trim().to_string()),
            None => (line.to_string(), format!("/home/dev/{line}")),
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

pub fn cache_args(flavor: &str) -> Vec<String> {
    let mut out = Vec::new();
    if flavor != "claude" {
        out.push("-v".into());
        out.push(format!("sbx-mise-{flavor}:/home/dev/.local/share/mise"));
        out.push("-v".into());
        out.push(format!(
            "sbx-mise-state-{flavor}:/home/dev/.local/state/mise"
        ));
    }
    for entry in cache_entries(flavor) {
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
    if flavor == "claude" {
        out.push("-v".into());
        out.push("sbx-claude-local:/home/dev/.local".into());
    }
    out
}

pub fn flavor_volumes(flavor: &str) -> Vec<String> {
    let mut v: Vec<String> = match flavor {
        "claude" => vec!["sbx-claude-local".into()],
        _ => vec![],
    };
    if flavor != "claude" {
        v.push(format!("sbx-mise-{flavor}"));
        v.push(format!("sbx-mise-state-{flavor}"));
    }
    for entry in cache_entries(flavor) {
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

pub fn image_up_to_date(flavor: &str) -> bool {
    let img = image_name(flavor);
    if !docker::image_exists(&img) {
        return false;
    }
    let img_mt = match docker::image_created_secs(&img) {
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
    Ok(())
}

pub fn resolve_image(flavor: &str, project_root: &Path, no_cache: bool) -> String {
    if flavor_depends_on_base(flavor) && !docker::image_exists(&image_name(BASE_FLAVOR)) {
        build_image(BASE_FLAVOR, false);
    }
    let base = image_name(flavor);
    if !docker::image_exists(&base) {
        build_image(flavor, false);
    }

    let project_df = sbx_file(project_root, "Dockerfile");
    if !project_df.is_file() {
        return base;
    }

    let img = project_image_tag(flavor, project_root);
    let needs_build = if !docker::image_exists(&img) {
        true
    } else {
        let df_mtime = mtime_secs(&project_df).unwrap_or(0);
        let img_mtime = docker::image_created_secs(&img).unwrap_or(0);
        df_mtime > img_mtime
    };
    if no_cache || needs_build {
        log(format!(
            "building project image {img} from {}",
            project_df.display()
        ));
        let uid = nix_uid();
        let gid = nix_gid();
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
