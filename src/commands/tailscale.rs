use std::io::Write;
use std::path::Path;

use crate::env_file;
use crate::network::{self, ProjectNetwork, TailscaleConfig};
use crate::project::{project_flavor, project_name};
use crate::tailscale::{
    self, authkey_env, is_valid_profile_name, sidecar_attached_count, sidecar_name,
    sidecar_running, AUTHKEY_ENV_BASE,
};
use crate::util::{die, env_file_path, log};

pub enum Action {
    On(Option<String>),
    Off,
    Status,
    Auth(Option<String>),
    List,
    Rm(String),
}

pub fn run(cwd: &Path, action: Action) {
    match action {
        Action::On(name) => cmd_on(cwd, name.as_deref()),
        Action::Off => cmd_off(cwd),
        Action::Status => cmd_status(cwd),
        Action::Auth(name) => cmd_auth(name.as_deref()),
        Action::List => cmd_list(),
        Action::Rm(name) => cmd_rm(&name),
    }
}

fn validate_profile_or_die(name: &str) {
    if !is_valid_profile_name(name) {
        die(format!(
            "invalid profile name '{name}': use lowercase letters, digits, '-', '_'"
        ));
    }
}

fn cmd_on(cwd: &Path, name: Option<&str>) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
    let (value, env_name) = match name {
        None => ("1".to_string(), authkey_env(None)),
        Some(n) => {
            validate_profile_or_die(n);
            (n.to_string(), authkey_env(Some(n)))
        }
    };
    if std::env::var(&env_name).unwrap_or_default().is_empty() {
        let hint = match name {
            None => "sbx net tailscale auth".to_string(),
            Some(n) => format!("sbx net tailscale auth {n}"),
        };
        log(format!(
            "warning: {env_name} is not set — run '{hint}' before starting a shell"
        ));
    }
    if let Err(e) = network::set_key(&root, "tailscale", &value) {
        die(format!("write .sbx/network: {e}"));
    }
    let pname = project_name(&root);
    match name {
        None => log(format!("tailscale enabled for {pname} (default profile)")),
        Some(n) => log(format!("tailscale enabled for {pname} (profile: {n})")),
    }
}

fn cmd_off(cwd: &Path) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/flavor here. run 'sbx init <flavor>' first."));
    let prev = ProjectNetwork::read(&root).tailscale;
    if let Err(e) = network::set_key(&root, "tailscale", "") {
        die(format!("write .sbx/network: {e}"));
    }
    log(format!("tailscale disabled for {}", project_name(&root)));
    let profile = match &prev {
        TailscaleConfig::Named(n) => Some(n.as_str()),
        _ => None,
    };
    let sidecar = sidecar_name(&project_name(&root), profile);
    tailscale::stop_sidecar_if_idle(&sidecar);
}

fn cmd_status(cwd: &Path) {
    let profiles = configured_profiles();
    if profiles.is_empty() {
        log(format!(
            "no auth keys saved (run 'sbx net tailscale auth [name]')"
        ));
    } else {
        log(format!("saved profiles: {}", profiles.join(", ")));
    }
    let Some((_, root)) = project_flavor(cwd) else {
        log("not in a project");
        return;
    };
    let pname = project_name(&root);
    let net = ProjectNetwork::read(&root);
    match &net.tailscale {
        TailscaleConfig::Disabled => {
            log(format!("project {pname}: tailscale disabled"));
            return;
        }
        TailscaleConfig::Default => log(format!("project {pname}: tailscale=1 (default profile)")),
        TailscaleConfig::Named(n) => log(format!("project {pname}: tailscale={n}")),
    }
    let profile = net.tailscale.profile();
    let env_name = authkey_env(profile);
    let has_key = !std::env::var(&env_name).unwrap_or_default().is_empty();
    log(format!(
        "  {env_name}: {}",
        if has_key { "set" } else { "UNSET" }
    ));
    let sidecar = sidecar_name(&pname, profile);
    if sidecar_running(&sidecar) {
        let n = sidecar_attached_count(&sidecar);
        log(format!("  sidecar: {sidecar} ({n} attached)"));
    }
}

fn cmd_auth(name: Option<&str>) {
    if let Some(n) = name {
        validate_profile_or_die(n);
    }
    let env_name = authkey_env(name);
    let label = match name {
        None => "default profile".to_string(),
        Some(n) => format!("profile '{n}'"),
    };
    eprint!("tailscale auth key for {label} (tskey-...): ");
    let _ = std::io::stderr().flush();
    let key = read_line_hidden();
    eprintln!();
    if key.is_empty() {
        die("empty auth key; aborting");
    }
    if !key.starts_with("tskey-") {
        log("warning: auth key doesn't start with 'tskey-' — saving anyway");
    }
    if let Err(e) = env_file::set_var(&env_name, &key) {
        die(format!("write env file: {e}"));
    }
    log(format!("saved {env_name} to ~/.config/sbx/env (chmod 600)"));
}

fn cmd_list() {
    let profiles = configured_profiles();
    if profiles.is_empty() {
        log("no tailscale profiles saved");
        return;
    }
    for p in profiles {
        println!("{p}");
    }
}

fn cmd_rm(name: &str) {
    let env_name = if name == "default" || name.is_empty() {
        authkey_env(None)
    } else {
        validate_profile_or_die(name);
        authkey_env(Some(name))
    };
    if std::env::var(&env_name).unwrap_or_default().is_empty() {
        die(format!("no such profile: {name}"));
    }
    if let Err(e) = env_file::unset_var(&env_name) {
        die(format!("unset {env_name}: {e}"));
    }
    log(format!("removed {env_name}"));
}

fn configured_profiles() -> Vec<String> {
    let entries = env_file::parse_env_file(&env_file_path());
    let mut out = Vec::new();
    for e in entries {
        if e.key == AUTHKEY_ENV_BASE {
            out.push("default".to_string());
        } else if let Some(rest) = e.key.strip_prefix(&format!("{AUTHKEY_ENV_BASE}_")) {
            out.push(rest.to_ascii_lowercase());
        }
    }
    out.sort();
    out
}

fn read_line_hidden() -> String {
    use std::os::unix::io::AsRawFd;
    let stdin = std::io::stdin();
    let fd = stdin.as_raw_fd();
    let mut term = unsafe { std::mem::zeroed::<libc_compat::termios>() };
    let had_attr = unsafe { libc_compat::tcgetattr(fd, &mut term) } == 0;
    let original = term;
    if had_attr {
        term.c_lflag &= !libc_compat::ECHO;
        unsafe { libc_compat::tcsetattr(fd, libc_compat::TCSANOW, &term) };
    }
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    if had_attr {
        unsafe { libc_compat::tcsetattr(fd, libc_compat::TCSANOW, &original) };
    }
    s.trim_end_matches(['\r', '\n']).to_string()
}

mod libc_compat {
    #[repr(C)]
    #[derive(Copy, Clone)]
    pub struct termios {
        pub c_iflag: u32,
        pub c_oflag: u32,
        pub c_cflag: u32,
        pub c_lflag: u32,
        pub c_line: u8,
        pub c_cc: [u8; 32],
        pub c_ispeed: u32,
        pub c_ospeed: u32,
    }
    pub const ECHO: u32 = 0o000010;
    pub const TCSANOW: i32 = 0;
    unsafe extern "C" {
        pub fn tcgetattr(fd: i32, termios: *mut termios) -> i32;
        pub fn tcsetattr(fd: i32, optional_actions: i32, termios: *const termios) -> i32;
    }
}

pub fn list_profiles() -> Vec<String> {
    configured_profiles()
}
