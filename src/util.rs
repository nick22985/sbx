use std::io::Write;
use std::path::{Path, PathBuf};

pub fn home_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| home_dir().join(".config"))
        .join("sbx")
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
