use std::fs;
use std::path::Path;

use crate::project::{sbx_file, sbx_write_dir};
use crate::util::log;

#[derive(Default, Debug, Clone)]
pub struct ProjectNetwork {
    pub vpn: Option<String>,
    pub tailscale: TailscaleConfig,
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
pub enum TailscaleConfig {
    #[default]
    Disabled,
    Default,
    Named(String),
}

impl TailscaleConfig {
    pub fn enabled(&self) -> bool {
        !matches!(self, Self::Disabled)
    }
    pub fn profile(&self) -> Option<&str> {
        match self {
            Self::Named(n) => Some(n.as_str()),
            _ => None,
        }
    }
    fn parse(value: &str) -> Self {
        match value {
            "" | "0" | "false" | "no" | "off" => Self::Disabled,
            "1" | "true" | "yes" | "on" => Self::Default,
            name => Self::Named(name.to_string()),
        }
    }
}

impl ProjectNetwork {
    pub fn read(project_root: &Path) -> Self {
        let mut out = Self::default();
        let net = sbx_file(project_root, "network");
        let Ok(content) = fs::read_to_string(&net) else {
            return out;
        };
        for raw in content.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                log(format!("ignoring malformed line in .sbx/network: {raw}"));
                continue;
            };
            let key = k.trim();
            let value = v.trim();
            match key {
                "vpn" => {
                    out.vpn = if value.is_empty() {
                        None
                    } else {
                        Some(value.to_string())
                    }
                }
                "tailscale" => out.tailscale = TailscaleConfig::parse(value),
                _ => log(format!("unknown key in .sbx/network: {key}")),
            }
        }
        out
    }
}

pub fn set_key(project_root: &Path, key: &str, value: &str) -> std::io::Result<()> {
    let dir = sbx_write_dir(project_root);
    fs::create_dir_all(&dir)?;
    let path = dir.join("network");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let mut out = String::new();
    let mut replaced = false;
    for raw in existing.lines() {
        let stripped = raw.split('#').next().unwrap_or("").trim();
        if let Some((k, _)) = stripped.split_once('=')
            && k.trim() == key
        {
            if !value.is_empty() && !replaced {
                out.push_str(&format!("{key}={value}\n"));
                replaced = true;
            }
            continue;
        }
        out.push_str(raw);
        out.push('\n');
    }
    if !value.is_empty() && !replaced {
        out.push_str(&format!("{key}={value}\n"));
    }
    if out.trim().is_empty() {
        let _ = fs::remove_file(&path);
        return Ok(());
    }
    fs::write(&path, out)
}
