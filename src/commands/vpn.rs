use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::network;
use crate::project::{project_flavor, project_name};
use crate::util::{die, log};
use crate::vpn::{
    self, project_vpn_spec, resolve_ovpn, sidecar_attached_count, sidecar_name, sidecar_running,
};

pub enum Action {
    Status,
    Use(String),
    Auth,
    Inline(Option<PathBuf>),
    Off,
}

pub fn run(cwd: &Path, action: Action) {
    match action {
        Action::Use(spec) => cmd_use(cwd, &spec),
        Action::Auth => cmd_auth(cwd),
        Action::Inline(target) => cmd_inline(cwd, target),
        Action::Off => cmd_off(cwd),
        Action::Status => cmd_status(cwd),
    }
}

fn cmd_use(cwd: &Path, spec: &str) {
    let resolved = resolve_ovpn(spec).unwrap_or_else(|| {
        die(format!(
            "'{spec}' is a bare name but SBX_VPN_DIR is not set"
        ))
    });
    if !resolved.is_file() {
        die(format!("no such file: {}", resolved.display()));
    }
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));
    if let Err(e) = network::set_key(&root, "vpn", spec) {
        die(format!("write config.toml: {e}"));
    }
    log(format!("vpn={spec} (resolved: {})", resolved.display()));
}

fn cmd_auth(cwd: &Path) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));
    let spec = project_vpn_spec(&root)
        .unwrap_or_else(|| die("no vpn configured (run 'sbx net vpn use ...' first)"));
    let resolved = resolve_ovpn(&spec)
        .unwrap_or_else(|| die("cannot resolve - set SBX_VPN_DIR or use a full path"));
    let mut auth_path = resolved.as_os_str().to_owned();
    auth_path.push(".auth");
    let auth_path = PathBuf::from(auth_path);

    eprint!("VPN username: ");
    let _ = std::io::stderr().flush();
    let user = read_line_visible();
    eprint!("VPN password: ");
    let _ = std::io::stderr().flush();
    let pass = read_line_hidden();
    eprintln!();

    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&auth_path)
        .unwrap_or_else(|e| die(format!("open {}: {e}", auth_path.display())));
    let _ = writeln!(f, "{user}");
    let _ = writeln!(f, "{pass}");
    log(format!(
        "saved credentials to {} (chmod 600)",
        auth_path.display()
    ));
}

fn cmd_inline(cwd: &Path, target: Option<PathBuf>) {
    let target = match target {
        Some(t) => t,
        None => {
            let (_, root) = project_flavor(cwd)
                .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));
            let spec = project_vpn_spec(&root)
                .unwrap_or_else(|| die("no vpn configured (run 'sbx net vpn use ...' first)"));
            resolve_ovpn(&spec)
                .unwrap_or_else(|| die("cannot resolve - set SBX_VPN_DIR or use a full path"))
        }
    };
    if !target.is_file() {
        die(format!("no such file: {}", target.display()));
    }
    match vpn::inline_ovpn(&target) {
        Ok(stats) => {
            log(format!(
                "inlined {} reference(s) into {}",
                stats.replaced,
                target.display()
            ));
            if stats.unquoted > 0 {
                log(format!(
                    "stripped nmcli single-quotes from {} line(s)",
                    stats.unquoted
                ));
            }
            if stats.dropped > 0 {
                log(format!(
                    "dropped/rewrote {} host-side directive(s)",
                    stats.dropped
                ));
            }
            if stats.missing > 0 {
                log(format!(
                    "warning: {} referenced file(s) were missing",
                    stats.missing
                ));
            }
        }
        Err(e) => die(e),
    }
}

fn cmd_off(cwd: &Path) {
    let (_, root) = project_flavor(cwd)
        .unwrap_or_else(|| die("no .sbx/config.toml here. run 'sbx init <flavor>' first."));
    if let Err(e) = network::set_key(&root, "vpn", "") {
        die(format!("write config.toml: {e}"));
    }
    log("cleared vpn from config.toml");
}

fn cmd_status(cwd: &Path) {
    match std::env::var("SBX_VPN_DIR") {
        Ok(d) => log(format!("SBX_VPN_DIR={d}")),
        Err(_) => log("SBX_VPN_DIR not set - use absolute paths in config.toml"),
    }
    let Some((_, root)) = project_flavor(cwd) else {
        log("not in a project");
        return;
    };
    let pname = project_name(&root);
    let Some(spec) = project_vpn_spec(&root) else {
        log(format!("project {pname}: no vpn configured"));
        return;
    };
    log(format!("project {pname}: vpn={spec}"));
    match resolve_ovpn(&spec) {
        Some(p) if p.is_file() => {
            log(format!("  resolved: {} (ok)", p.display()));
            let mut auth = p.as_os_str().to_owned();
            auth.push(".auth");
            let auth = PathBuf::from(auth);
            if auth.is_file() {
                log(format!("  auth: {}", auth.display()));
            }
        }
        Some(p) => log(format!("  resolved: {} (MISSING)", p.display())),
        None => log("  cannot resolve (SBX_VPN_DIR not set?)"),
    }
    let sidecar = sidecar_name(&spec);
    if sidecar_running(&sidecar) {
        let n = sidecar_attached_count(&sidecar);
        log(format!("sidecar: {sidecar} ({n} attached)"));
    }
}

fn read_line_visible() -> String {
    let mut s = String::new();
    let _ = std::io::stdin().read_line(&mut s);
    s.trim_end_matches(['\r', '\n']).to_string()
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
