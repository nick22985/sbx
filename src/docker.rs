use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::project::{project_name, sbx_file};
use crate::util::{home_dir, log};

pub const FORWARDED_VARS: &[&str] = &[
    "SBX_RUN_SCRIPTS",
    "SBX_SCANNERS",
    "SOCKET_API_KEY",
    "SOCKET_CLI_API_TOKEN",
    "SOCKET_ORG_SLUG",
];

pub fn image_exists(image: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", image])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn image_created_secs(image: &str) -> Option<u64> {
    let out = Command::new("docker")
        .args(["image", "inspect", image, "--format", "{{.Created}}"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&out.stdout).trim().to_string();
    parse_docker_time(&raw)
}

fn parse_docker_time(s: &str) -> Option<u64> {
    let out = Command::new("date").args(["-d", s, "+%s"]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

pub fn stdio_is_tty() -> bool {
    unsafe { libc_compat::isatty(0) != 0 && libc_compat::isatty(1) != 0 }
}

mod libc_compat {
    unsafe extern "C" {
        pub fn isatty(fd: i32) -> i32;
    }
}

pub fn find_running_container(flavor: &str, pname: &str) -> Option<String> {
    let out = Command::new("docker")
        .args([
            "ps",
            "--filter",
            &format!("name=^sbx-{flavor}-{pname}-"),
            "--format",
            "{{.Names}}",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|s| s.to_string())
}

pub fn bridge_subnet() -> String {
    container_inspect_bridge("Subnet").unwrap_or_else(|| "172.17.0.0/16".to_string())
}

pub fn bridge_gateway() -> String {
    container_inspect_bridge("Gateway").unwrap_or_else(|| "172.17.0.1".to_string())
}

fn container_inspect_bridge(field: &str) -> Option<String> {
    let out = Command::new("docker")
        .args([
            "network",
            "inspect",
            "bridge",
            "--format",
            &format!("{{{{(index .IPAM.Config 0).{field}}}}}"),
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[derive(Clone, Copy)]
pub enum Network<'a> {
    Bridge,
    ShareWith(&'a str),
}

#[derive(Default)]
pub struct PortSpec {
    pub ports: Vec<u16>,
}

impl PortSpec {
    pub fn from_project(project_root: &Path) -> Self {
        let mut seen = std::collections::BTreeSet::new();
        let mut ports = Vec::new();
        let ports_file = sbx_file(project_root, "ports");
        if let Ok(contents) = std::fs::read_to_string(&ports_file) {
            for raw in contents.lines() {
                if let Some(p) = parse_port_line(raw)
                    && seen.insert(p)
                {
                    ports.push(p);
                }
            }
        }
        if let Ok(extra) = std::env::var("SBX_PORTS") {
            for raw in extra.split(',') {
                if let Some(p) = parse_port_line(raw)
                    && seen.insert(p)
                {
                    ports.push(p);
                }
            }
        }
        PortSpec { ports }
    }

    pub fn to_docker_args(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.ports.is_empty() {
            return out;
        }
        log(format!(
            "forwarding 127.0.0.1 -> container: {}",
            self.ports
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(" ")
        ));
        log("hint: dev server must bind to 0.0.0.0 in the container (e.g. vite --host, HOST=0.0.0.0)");
        for p in &self.ports {
            out.push("-p".to_string());
            out.push(format!("127.0.0.1:{p}:{p}"));
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.ports.is_empty()
    }
}

fn parse_port_line(raw: &str) -> Option<u16> {
    let no_comment = raw.split('#').next()?;
    let cleaned: String = no_comment.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.is_empty() {
        return None;
    }
    match cleaned.parse::<u16>() {
        Ok(p) => Some(p),
        Err(_) => {
            log(format!("ignoring invalid port: {cleaned}"));
            None
        }
    }
}

pub fn worktree_mount_args(project_root: &Path) -> Vec<String> {
    let dotgit = project_root.join(".git");
    if !dotgit.is_file() {
        return Vec::new();
    }
    let Ok(content) = std::fs::read_to_string(&dotgit) else {
        return Vec::new();
    };
    let mut gitdir: Option<PathBuf> = None;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("gitdir: ") {
            gitdir = Some(PathBuf::from(rest.trim()));
            break;
        }
    }
    let Some(mut gd) = gitdir else {
        return Vec::new();
    };
    if !gd.is_absolute() {
        gd = project_root.join(gd);
    }
    if !gd.is_dir() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let gd_s = gd.display().to_string();
    out.push("-v".into());
    out.push(format!("{gd_s}:{gd_s}"));
    let commondir_file = gd.join("commondir");
    if let Ok(rel) = std::fs::read_to_string(&commondir_file) {
        let rel = rel.lines().next().unwrap_or("").trim();
        if !rel.is_empty() {
            let mut cd = PathBuf::from(rel);
            if !cd.is_absolute() {
                cd = std::fs::canonicalize(gd.join(rel)).unwrap_or_else(|_| gd.join(rel));
            }
            if cd.is_dir() && cd != gd {
                let cd_s = cd.display().to_string();
                out.push("-v".into());
                out.push(format!("{cd_s}:{cd_s}"));
            }
        }
    }
    out
}

pub fn project_ssh_enabled(project_root: &Path) -> bool {
    let f = sbx_file(project_root, "ssh");
    f.is_file()
}

pub fn ssh_mount_args(project_root: &Path, container_home: &Path) -> Vec<String> {
    if !project_ssh_enabled(project_root) {
        return Vec::new();
    }
    let Ok(sock) = std::env::var("SSH_AUTH_SOCK") else {
        log(".sbx/ssh set but SSH_AUTH_SOCK is empty on host; skipping agent forward");
        return Vec::new();
    };
    let meta = match std::fs::metadata(&sock) {
        Ok(m) => m,
        Err(_) => {
            log(format!("SSH_AUTH_SOCK={sock} is not accessible; skipping"));
            return Vec::new();
        }
    };
    use std::os::unix::fs::FileTypeExt;
    if !meta.file_type().is_socket() {
        log(format!("SSH_AUTH_SOCK={sock} is not a socket; skipping"));
        return Vec::new();
    }
    let ch = container_home.display();
    let mut out = vec![
        "-v".into(),
        format!("{sock}:/ssh-agent"),
        "-e".into(),
        "SSH_AUTH_SOCK=/ssh-agent".into(),
    ];
    let home = home_dir();
    let kh = home.join(".ssh/known_hosts");
    if kh.is_file() {
        out.push("-v".into());
        out.push(format!("{}:{ch}/.ssh/known_hosts:ro", kh.display()));
    }
    let cfg = home.join(".ssh/config");
    if cfg.is_file() {
        out.push("-v".into());
        out.push(format!("{}:{ch}/.ssh/config:ro", cfg.display()));
    }
    out
}

pub struct RunSpec<'a> {
    pub image: &'a str,
    pub flavor: &'a str,
    pub project_root: &'a Path,
    pub entry: Vec<String>,
    pub network: Network<'a>,
    pub use_hostname: bool,
    pub publish_ports: PortSpec,
    pub extra_host_args: Vec<String>,
    pub host_workspace: bool,
    pub extra_mounts: Vec<PathBuf>,
    pub container_home: PathBuf,
}

pub fn run_container(spec: RunSpec<'_>) -> i32 {
    let pname = project_name(spec.project_root);
    let mut cmd = Command::new("docker");
    cmd.arg("run").arg("--rm");

    if stdio_is_tty() {
        cmd.args(["-i", "-t"]);
    } else {
        cmd.arg("-i");
    }
    if spec.use_hostname {
        cmd.args(["--hostname", &format!("sbx-{}", spec.flavor)]);
    }
    let pid = std::process::id();
    cmd.args(["--name", &format!("sbx-{}-{pname}-{pid}", spec.flavor)]);
    let uid = crate::flavor::nix_uid();
    let gid = crate::flavor::nix_gid();
    cmd.args(["--user", &format!("{uid}:{gid}")]);
    cmd.args(["--cap-drop=ALL", "--security-opt=no-new-privileges"]);
    for arg in &spec.extra_host_args {
        cmd.arg(arg);
    }
    match spec.network {
        Network::Bridge => {
            cmd.arg("--add-host=host.docker.internal:host-gateway");
        }
        Network::ShareWith(name) => {
            cmd.args(["--network", &format!("container:{name}")]);
        }
    }
    let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".into());
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();
    cmd.args(["-e", &format!("TERM={term}")]);
    cmd.args(["-e", &format!("COLORTERM={colorterm}")]);
    cmd.args(["-e", &format!("HOME={}", spec.container_home.display())]);
    for v in FORWARDED_VARS {
        if let Ok(val) = std::env::var(v)
            && !val.is_empty()
        {
            cmd.args(["-e", &format!("{v}={val}")]);
        }
    }
    let workspace = if spec.host_workspace {
        spec.project_root.display().to_string()
    } else {
        "/workspace".to_string()
    };
    cmd.args(["-w", &workspace]);
    cmd.arg("-v")
        .arg(format!("{}:{workspace}", spec.project_root.display()));
    for arg in worktree_mount_args(spec.project_root) {
        cmd.arg(arg);
    }
    for arg in ssh_mount_args(spec.project_root, &spec.container_home) {
        cmd.arg(arg);
    }
    for path in &spec.extra_mounts {
        if !path.is_dir() {
            log(format!(
                "skipping extra mount (not a directory): {}",
                path.display()
            ));
            continue;
        }
        let p = path.display().to_string();
        log(format!("mounting extra path: {p}"));
        cmd.arg("-v").arg(format!("{p}:{p}"));
    }
    for arg in crate::flavor::cache_args(spec.flavor) {
        cmd.arg(arg);
    }
    for arg in spec.publish_ports.to_docker_args() {
        cmd.arg(arg);
    }
    cmd.arg(spec.image);
    for a in &spec.entry {
        cmd.arg(a);
    }

    match cmd.status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            log(format!("docker: {e}"));
            1
        }
    }
}

pub fn exec_into(container: &str, entry: &[String]) -> io::Error {
    let mut cmd = Command::new("docker");
    cmd.arg("exec");
    if stdio_is_tty() {
        cmd.args(["-i", "-t"]);
    } else {
        cmd.arg("-i");
    }
    let term = std::env::var("TERM").unwrap_or_else(|_| "xterm-256color".into());
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();
    cmd.args(["-e", &format!("TERM={term}")]);
    cmd.args(["-e", &format!("COLORTERM={colorterm}")]);
    for v in FORWARDED_VARS {
        if let Ok(val) = std::env::var(v)
            && !val.is_empty()
        {
            cmd.args(["-e", &format!("{v}={val}")]);
        }
    }
    cmd.args(["-w", "/workspace"]);
    cmd.arg(container);
    let entry: &[String] = if entry.is_empty() {
        static SHELL: &[String] = &[];
        let _ = SHELL;
        &[]
    } else {
        entry
    };
    if entry.is_empty() {
        cmd.arg("/bin/bash");
    } else {
        for a in entry {
            cmd.arg(a);
        }
    }
    log(format!("attaching to running container: {container}"));
    cmd.exec()
}
