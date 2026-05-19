use std::io;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::project::{project_name, sbx_file};
use crate::util::{confirm, die, home_dir, log};

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
        pub fn signal(signum: i32, handler: usize) -> usize;
    }
}

extern "C" fn signal_nop(_: i32) {}

fn shield_parent_signals() {
    const SIGHUP: i32 = 1;
    const SIGINT: i32 = 2;
    const SIGTERM: i32 = 15;
    unsafe {
        libc_compat::signal(SIGHUP, signal_nop as *const () as usize);
        libc_compat::signal(SIGINT, signal_nop as *const () as usize);
        libc_compat::signal(SIGTERM, signal_nop as *const () as usize);
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

pub fn container_exists(name: &str, include_stopped: bool) -> bool {
    let mut cmd = Command::new("docker");
    cmd.arg("ps");
    if include_stopped {
        cmd.arg("-a");
    }
    cmd.args([
        "--filter",
        &format!("name=^{name}$"),
        "--format",
        "{{.Names}}",
    ]);
    let Ok(out) = cmd.output() else {
        return false;
    };
    !String::from_utf8_lossy(&out.stdout).trim().is_empty()
}

pub fn force_rm(name: &str) {
    let _ = Command::new("docker")
        .args(["rm", "-f", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Returns true if a container with the given name existed and was removed,
/// false if it was already absent.
pub fn stop_if_present(name: &str) -> bool {
    if !container_exists(name, true) {
        return false;
    }
    force_rm(name);
    true
}

/// Human-readable one-liner: "running (NAME)" / "stopped (NAME)" / "not present (NAME)".
pub fn state_line(name: &str) -> String {
    if container_exists(name, false) {
        format!("running ({name})")
    } else if container_exists(name, true) {
        format!("stopped ({name})")
    } else {
        format!("not present ({name})")
    }
}

pub fn tail_logs(name: &str, follow: bool) {
    let mut cmd = Command::new("docker");
    cmd.arg("logs");
    if follow {
        cmd.arg("-f");
    } else {
        cmd.args(["--tail", "200"]);
    }
    cmd.arg(name);
    let _ = cmd.status();
}

/// Count running containers sharing the given sidecar's network namespace
/// (i.e. started with `--network=container:<sidecar>`).
///
/// `docker ps --filter network=container:NAME` does not work — Docker's
/// network filter doesn't honor the `container:` syntax. We have to inspect
/// each running container's `HostConfig.NetworkMode` and match against the
/// sidecar's resolved container ID.
pub fn netns_attached_count(sidecar: &str) -> u32 {
    let Some(id) = container_id(sidecar) else {
        return 0;
    };
    let target = format!("container:{id}");
    let Ok(out) = Command::new("docker").args(["ps", "-q"]).output() else {
        return 0;
    };
    let ids: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if ids.is_empty() {
        return 0;
    }
    let mut args: Vec<String> = vec![
        "inspect".into(),
        "--format".into(),
        "{{.HostConfig.NetworkMode}}".into(),
    ];
    args.extend(ids);
    let Ok(out) = Command::new("docker").args(&args).output() else {
        return 0;
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| l.trim() == target)
        .count() as u32
}

fn container_id(name: &str) -> Option<String> {
    let out = Command::new("docker")
        .args(["inspect", "--format", "{{.Id}}", name])
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
    UserDefined(&'a str),
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

pub fn project_docker_enabled(project_root: &Path) -> bool {
    let f = sbx_file(project_root, "docker");
    f.is_file()
}

pub fn project_gui_enabled(project_root: &Path) -> bool {
    let f = sbx_file(project_root, "gui");
    f.is_file()
}

pub fn gui_mount_args(project_root: &Path) -> Vec<String> {
    if !project_gui_enabled(project_root) {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let mut forwarded = Vec::new();

    if let Ok(rt) = std::env::var("XDG_RUNTIME_DIR")
        && !rt.is_empty()
    {
        let rt_path = PathBuf::from(&rt);
        if rt_path.is_dir() {
            let uid = crate::flavor::nix_uid();
            let container_rt = format!("/run/user/{uid}");
            out.push("-v".into());
            out.push(format!("{}:{container_rt}", rt_path.display()));
            out.push("-e".into());
            out.push(format!("XDG_RUNTIME_DIR={container_rt}"));
            forwarded.push("XDG_RUNTIME_DIR".to_string());

            if let Ok(w) = std::env::var("WAYLAND_DISPLAY")
                && !w.is_empty()
            {
                out.push("-e".into());
                out.push(format!("WAYLAND_DISPLAY={w}"));
                forwarded.push(format!("WAYLAND_DISPLAY={w}"));
            }
            if let Ok(bus) = std::env::var("DBUS_SESSION_BUS_ADDRESS")
                && !bus.is_empty()
            {
                out.push("-e".into());
                out.push(format!("DBUS_SESSION_BUS_ADDRESS={bus}"));
                forwarded.push("DBUS_SESSION_BUS_ADDRESS".to_string());
            }
        }
    }

    if let Ok(display) = std::env::var("DISPLAY")
        && !display.is_empty()
    {
        let x11 = Path::new("/tmp/.X11-unix");
        if x11.is_dir() {
            out.push("-v".into());
            out.push("/tmp/.X11-unix:/tmp/.X11-unix".into());
            out.push("-e".into());
            out.push(format!("DISPLAY={display}"));
            forwarded.push(format!("DISPLAY={display}"));
        }
        let xauth_path = std::env::var("XAUTHORITY")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".Xauthority"));
        if xauth_path.is_file() {
            out.push("-v".into());
            out.push(format!("{}:/tmp/.Xauthority:ro", xauth_path.display()));
            out.push("-e".into());
            out.push("XAUTHORITY=/tmp/.Xauthority".into());
        }
    }

    let dri = Path::new("/dev/dri");
    if dri.exists() {
        out.push("--device".into());
        out.push("/dev/dri".into());
        if let Some(gid) = render_or_video_gid() {
            out.push("--group-add".into());
            out.push(gid.to_string());
        }
    }

    if forwarded.is_empty() {
        log(".sbx/gui is enabled but no host DISPLAY/WAYLAND_DISPLAY/XDG_RUNTIME_DIR was found");
    } else {
        log(format!("gui forwarding: {}", forwarded.join(", ")));
    }
    out
}

fn render_or_video_gid() -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    for node in ["/dev/dri/renderD128", "/dev/dri/card0"] {
        if let Ok(m) = std::fs::metadata(node) {
            return Some(m.gid());
        }
    }
    None
}

pub fn host_docker_socket() -> PathBuf {
    if let Ok(h) = std::env::var("DOCKER_HOST")
        && let Some(rest) = h.strip_prefix("unix://")
        && !rest.is_empty()
    {
        return PathBuf::from(rest);
    }
    PathBuf::from("/var/run/docker.sock")
}

fn socket_gid(sock: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(sock).ok().map(|m| m.gid())
}

/// SECURITY: this is effectively root on the host — anything inside can
/// `docker run --privileged -v /:/host …` to escape. Opt-in only.
pub fn docker_socket_mount_args() -> Vec<String> {
    let sock = host_docker_socket();
    use std::os::unix::fs::FileTypeExt;
    match std::fs::metadata(&sock) {
        Ok(m) if m.file_type().is_socket() => {}
        Ok(_) => {
            log(format!(
                "docker socket {} is not a socket; skipping mount",
                sock.display()
            ));
            return Vec::new();
        }
        Err(e) => {
            log(format!(
                "docker socket {} not accessible ({e}); skipping mount",
                sock.display()
            ));
            return Vec::new();
        }
    }
    let sock_s = sock.display().to_string();
    let mut out = vec!["-v".into(), format!("{sock_s}:/var/run/docker.sock")];
    if let Some(gid) = socket_gid(&sock) {
        out.push("--group-add".into());
        out.push(gid.to_string());
    }
    log(format!(
        "mounting host docker socket: {sock_s} (host root inside container)"
    ));
    out
}

enum AgentStatus {
    HasKeys,
    NoKeys,
    Unreachable,
}

fn ssh_add_status(sock: &str) -> AgentStatus {
    let status = Command::new("ssh-add")
        .arg("-l")
        .env("SSH_AUTH_SOCK", sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    match status.ok().and_then(|s| s.code()) {
        Some(0) => AgentStatus::HasKeys,
        Some(1) => AgentStatus::NoKeys,
        _ => AgentStatus::Unreachable,
    }
}

pub fn ensure_ssh_agent_ready(project_root: &Path) {
    if !project_ssh_enabled(project_root) {
        return;
    }
    let sock = match std::env::var("SSH_AUTH_SOCK") {
        Ok(s) if !s.is_empty() => s,
        _ => die(
            ".sbx/ssh is enabled but SSH_AUTH_SOCK is empty on host.\n  start an agent and load a key, e.g.:\n    eval \"$(ssh-agent)\" && ssh-add",
        ),
    };
    let meta = std::fs::metadata(&sock).unwrap_or_else(|e| {
        die(format!(
            "SSH_AUTH_SOCK={sock} not accessible ({e}). is the agent still running?"
        ))
    });
    use std::os::unix::fs::FileTypeExt;
    if !meta.file_type().is_socket() {
        die(format!("SSH_AUTH_SOCK={sock} is not a socket"));
    }
    match ssh_add_status(&sock) {
        AgentStatus::HasKeys => {}
        AgentStatus::NoKeys => {
            log("ssh-agent has no keys loaded");
            let prompt_ok = stdio_is_tty() && confirm("add default identities now? (ssh-add)");
            if !prompt_ok {
                die(
                    "aborting: no keys in agent. run `ssh-add` (or `ssh-add ~/.ssh/id_…`) and retry",
                );
            }
            let ran = Command::new("ssh-add").env("SSH_AUTH_SOCK", &sock).status();
            if !matches!(ran, Ok(s) if s.success()) {
                die("ssh-add failed; aborting");
            }
            if !matches!(ssh_add_status(&sock), AgentStatus::HasKeys) {
                die("ssh-add reported success but the agent still has no keys; aborting");
            }
        }
        AgentStatus::Unreachable => die(format!(
            "SSH_AUTH_SOCK={sock} but ssh-add can't talk to the agent (stale socket?)"
        )),
    }
}

pub fn ssh_mount_args(project_root: &Path, container_home: &Path) -> Vec<String> {
    if !project_ssh_enabled(project_root) {
        return Vec::new();
    }
    let sock = std::env::var("SSH_AUTH_SOCK").unwrap_or_else(|_| {
        die(".sbx/ssh is enabled but SSH_AUTH_SOCK is empty (call ensure_ssh_agent_ready first)")
    });
    let meta = std::fs::metadata(&sock)
        .unwrap_or_else(|e| die(format!("SSH_AUTH_SOCK={sock} not accessible: {e}")));
    use std::os::unix::fs::FileTypeExt;
    if !meta.file_type().is_socket() {
        die(format!("SSH_AUTH_SOCK={sock} is not a socket"));
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
    pub extra_mounts: Vec<crate::mounts::Mount>,
    pub container_home: PathBuf,
    pub labels: Vec<String>,
    pub mount_docker_socket: bool,
    pub extra_env: Vec<(String, String)>,
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
        Network::UserDefined(net) => {
            cmd.args(["--network", net]);
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
    for k in crate::env_file::forwarded_keys() {
        if FORWARDED_VARS.contains(&k.as_str()) {
            continue;
        }
        if let Ok(val) = std::env::var(&k) {
            cmd.args(["-e", &format!("{k}={val}")]);
        }
    }
    let workspace = spec.project_root.display().to_string();
    cmd.args(["-w", &workspace]);
    cmd.arg("-v").arg(format!("{workspace}:{workspace}"));
    cmd.args(["-e", &format!("SBX_PROJECT_DIR={workspace}")]);
    for (k, v) in &spec.extra_env {
        cmd.args(["-e", &format!("{k}={v}")]);
    }
    for arg in worktree_mount_args(spec.project_root) {
        cmd.arg(arg);
    }
    for arg in ssh_mount_args(spec.project_root, &spec.container_home) {
        cmd.arg(arg);
    }
    for arg in gui_mount_args(spec.project_root) {
        cmd.arg(arg);
    }
    if spec.mount_docker_socket {
        for arg in docker_socket_mount_args() {
            cmd.arg(arg);
        }
    }
    for m in &spec.extra_mounts {
        if !m.host.exists() {
            log(format!(
                "skipping extra mount (host path missing): {}",
                m.host.display()
            ));
            continue;
        }
        let host = m.host.display();
        let container = m.container.display();
        log(format!("mounting extra path: {host} -> {container}"));
        let spec = if m.ro {
            format!("{host}:{container}:ro")
        } else {
            format!("{host}:{container}")
        };
        cmd.arg("-v").arg(spec);
    }
    let user_mount_targets: std::collections::BTreeSet<PathBuf> = spec
        .extra_mounts
        .iter()
        .map(|m| m.container.clone())
        .collect();
    let cache = crate::flavor::cache_args(spec.flavor);
    let mut i = 0;
    while i < cache.len() {
        if cache[i] == "-v"
            && let Some(vol) = cache.get(i + 1)
        {
            let target = vol.splitn(3, ':').nth(1).map(PathBuf::from);
            if let Some(t) = target
                && user_mount_targets.contains(&t)
            {
                log(format!(
                    "skipping default cache volume for {} (overridden by user mount)",
                    t.display()
                ));
                i += 2;
                continue;
            }
            cmd.arg(&cache[i]);
            cmd.arg(vol);
            i += 2;
        } else {
            cmd.arg(&cache[i]);
            i += 1;
        }
    }
    for arg in spec.publish_ports.to_docker_args() {
        cmd.arg(arg);
    }
    for arg in &spec.labels {
        cmd.arg(arg);
    }
    cmd.arg(spec.image);
    for a in &spec.entry {
        cmd.arg(a);
    }

    shield_parent_signals();
    match cmd.status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            log(format!("docker: {e}"));
            1
        }
    }
}

pub fn exec_into(container: &str, project_root: &Path, entry: &[String]) -> io::Error {
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
    for k in crate::env_file::forwarded_keys() {
        if FORWARDED_VARS.contains(&k.as_str()) {
            continue;
        }
        if let Ok(val) = std::env::var(&k) {
            cmd.args(["-e", &format!("{k}={val}")]);
        }
    }
    cmd.args(["-w", &project_root.display().to_string()]);
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
