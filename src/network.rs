use std::path::Path;

use crate::config::Config;

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
        let cfg = Config::load_or_default(project_root);
        Self {
            vpn: cfg.network.vpn,
            tailscale: cfg
                .network
                .tailscale
                .as_deref()
                .map(TailscaleConfig::parse)
                .unwrap_or_default(),
        }
    }
}

pub fn set_key(project_root: &Path, key: &str, value: &str) -> std::io::Result<()> {
    Config::edit(project_root, |cfg| {
        let v = if value.is_empty() {
            None
        } else {
            Some(value.to_string())
        };
        match key {
            "vpn" => cfg.network.vpn = v,
            "tailscale" => cfg.network.tailscale = v,
            _ => {}
        }
    })?;
    Ok(())
}
