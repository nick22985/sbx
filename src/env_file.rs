use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::util::{env_file_path, log};

pub struct EnvEntry {
    pub key: String,
    pub value: String,
}

pub fn parse_env_file(path: &Path) -> Vec<EnvEntry> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for raw in content.lines() {
        let line = raw.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some(eq) = line.find('=') else {
            log(format!("ignoring malformed env line: {raw}"));
            continue;
        };
        let mut key = &line[..eq];
        let mut value = &line[eq + 1..];
        if let Some(rest) = key.strip_prefix("export ") {
            key = rest;
        }
        let key: String = key.chars().filter(|c| !c.is_whitespace()).collect();
        if !is_valid_name(&key) {
            log(format!("ignoring invalid var name: {key}"));
            continue;
        }
        if value.len() >= 2 {
            let first = value.as_bytes()[0];
            let last = value.as_bytes()[value.len() - 1];
            if (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'') {
                value = &value[1..value.len() - 1];
            }
        }
        out.push(EnvEntry {
            key,
            value: value.to_string(),
        });
    }
    out
}

pub fn is_valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn load_into_env() {
    let path = env_file_path();
    if !path.exists() {
        return;
    }
    if let Ok(meta) = fs::metadata(&path) {
        let mode = meta.permissions().mode() & 0o777;
        if mode != 0o600 && mode != 0o400 {
            log(format!(
                "warning: {} has perms {:o} (chmod 600 recommended)",
                path.display(),
                mode
            ));
        }
    }
    for entry in parse_env_file(&path) {
        if std::env::var_os(&entry.key).is_none() {
            // SAFETY: single-threaded process startup.
            unsafe {
                std::env::set_var(&entry.key, &entry.value);
            }
        }
    }
}

pub fn ensure_file() -> io::Result<()> {
    let path = env_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !path.exists() {
        fs::write(&path, "")?;
    }
    let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    Ok(())
}

pub fn mask_value(key: &str, val: &str) -> String {
    let up = key.to_ascii_uppercase();
    let sensitive = ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASS"]
        .iter()
        .any(|k| up.contains(k));
    if !sensitive {
        return val.to_string();
    }
    let n = val.chars().count();
    if n <= 4 {
        return "****".to_string();
    }
    let chars: Vec<char> = val.chars().collect();
    let head: String = chars[..2].iter().collect();
    let tail: String = chars[n - 2..].iter().collect();
    format!("{head}****{tail}")
}

pub fn set_var(key: &str, val: &str) -> io::Result<()> {
    if !is_valid_name(key) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid var name: {key}"),
        ));
    }
    ensure_file()?;
    let path = env_file_path();
    let mut existing = parse_env_file(&path);
    let mut replaced = false;
    for entry in existing.iter_mut() {
        if entry.key == key {
            entry.value = val.to_string();
            replaced = true;
        }
    }
    if !replaced {
        existing.push(EnvEntry {
            key: key.to_string(),
            value: val.to_string(),
        });
    }
    write_env(&path, &existing)
}

pub fn unset_var(key: &str) -> io::Result<()> {
    let path = env_file_path();
    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no {}", path.display()),
        ));
    }
    let entries = parse_env_file(&path)
        .into_iter()
        .filter(|e| e.key != key)
        .collect::<Vec<_>>();
    write_env(&path, &entries)
}

fn write_env(path: &Path, entries: &[EnvEntry]) -> io::Result<()> {
    use std::io::Write;
    let mut tmp = tempfile_in_same_dir(path)?;
    for e in entries {
        writeln!(tmp.file, "{}={}", e.key, e.value)?;
    }
    let _ = fs::set_permissions(&tmp.path, fs::Permissions::from_mode(0o600));
    fs::rename(&tmp.path, path)
}

struct Tmp {
    path: std::path::PathBuf,
    file: fs::File,
}

fn tempfile_in_same_dir(path: &Path) -> io::Result<Tmp> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| ".sbx-tmp".to_string());
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let tmp_path = dir.join(format!(".{stem}.{pid}.{nanos}.tmp"));
    let f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)?;
    Ok(Tmp {
        path: tmp_path,
        file: f,
    })
}
