use std::ffi::OsStr;

use clap::{Parser, Subcommand};
use clap_complete::engine::{ArgValueCompleter, CompletionCandidate};

use crate::env_file;
use crate::flavor::list_flavors;
use crate::project::{project_flavor, sbx_file};
use crate::service::BUILTIN_SERVICES;
use crate::util::{env_file_path, expand_tilde};

fn fuzzy(pattern: &str, target: &str) -> bool {
    let mut p = pattern.chars().peekable();
    for c in target.chars() {
        if let Some(&want) = p.peek() {
            if c.eq_ignore_ascii_case(&want) {
                p.next();
            }
        } else {
            return true;
        }
    }
    p.peek().is_none()
}

fn complete_flavor(current: &OsStr) -> Vec<CompletionCandidate> {
    let cur = current.to_str().unwrap_or("");
    list_flavors()
        .into_iter()
        .filter(|f| cur.is_empty() || fuzzy(cur, f))
        .map(CompletionCandidate::new)
        .collect()
}

fn complete_flavor_or_all(current: &OsStr) -> Vec<CompletionCandidate> {
    let cur = current.to_str().unwrap_or("");
    let mut out = complete_flavor(current);
    if cur.is_empty() || fuzzy(cur, "all") {
        out.push(CompletionCandidate::new("all"));
    }
    out
}

fn complete_service(current: &OsStr) -> Vec<CompletionCandidate> {
    let cur = current.to_str().unwrap_or("");
    BUILTIN_SERVICES
        .iter()
        .filter(|s| cur.is_empty() || fuzzy(cur, s))
        .map(|s| CompletionCandidate::new(*s))
        .collect()
}

fn complete_configured_service(current: &OsStr) -> Vec<CompletionCandidate> {
    let cur = current.to_str().unwrap_or("");
    let Ok(cwd) = std::env::current_dir() else {
        return Vec::new();
    };
    let Some((_, root)) = project_flavor(&cwd) else {
        return Vec::new();
    };
    let f = sbx_file(&root, "services");
    let Ok(content) = std::fs::read_to_string(&f) else {
        return Vec::new();
    };
    content
        .lines()
        .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty() && (cur.is_empty() || fuzzy(cur, s)))
        .map(CompletionCandidate::new)
        .collect()
}

fn complete_configured_port(current: &OsStr) -> Vec<CompletionCandidate> {
    let cur = current.to_str().unwrap_or("");
    let Ok(cwd) = std::env::current_dir() else {
        return Vec::new();
    };
    let Some((_, root)) = project_flavor(&cwd) else {
        return Vec::new();
    };
    let f = sbx_file(&root, "ports");
    let Ok(content) = std::fs::read_to_string(&f) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|l| {
            let cleaned: String = l
                .split('#')
                .next()
                .unwrap_or("")
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            if cleaned.is_empty() || cleaned.parse::<u16>().is_err() {
                return None;
            }
            if cur.is_empty() || fuzzy(cur, &cleaned) {
                Some(CompletionCandidate::new(cleaned))
            } else {
                None
            }
        })
        .collect()
}

fn complete_env_key(current: &OsStr) -> Vec<CompletionCandidate> {
    let cur = current.to_str().unwrap_or("");
    let path = env_file_path();
    env_file::parse_env_file(&path)
        .into_iter()
        .filter(|e| cur.is_empty() || fuzzy(cur, &e.key))
        .map(|e| CompletionCandidate::new(e.key))
        .collect()
}

fn complete_claude_profile(current: &OsStr) -> Vec<CompletionCandidate> {
    let cur = current.to_str().unwrap_or("");
    crate::commands::claude::list_profiles()
        .into_iter()
        .filter(|p| cur.is_empty() || fuzzy(cur, p))
        .map(CompletionCandidate::new)
        .collect()
}

fn complete_tailscale_profile(current: &OsStr) -> Vec<CompletionCandidate> {
    let cur = current.to_str().unwrap_or("");
    crate::commands::tailscale::list_profiles()
        .into_iter()
        .filter(|p| cur.is_empty() || fuzzy(cur, p))
        .map(CompletionCandidate::new)
        .collect()
}

fn complete_vpn_name(current: &OsStr) -> Vec<CompletionCandidate> {
    let cur = current.to_str().unwrap_or("");
    let Ok(dir_raw) = std::env::var("SBX_VPN_DIR") else {
        return Vec::new();
    };
    let dir = expand_tilde(&dir_raw);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let stem = name.strip_suffix(".ovpn")?;
            if !cur.is_empty() && !fuzzy(cur, stem) {
                return None;
            }
            Some(CompletionCandidate::new(stem))
        })
        .collect()
}

fn complete_top_level(current: &OsStr) -> Vec<CompletionCandidate> {
    complete_flavor(current)
}

#[derive(Parser)]
#[command(
    name = "sbx",
    version,
    about = "Sandboxed Docker dev environments for npm/bun/rust/claude",
    subcommand_negates_reqs = true,
    arg_required_else_help = false
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Cmd>,

    /// Ad-hoc: `sbx <flavor>` enters a transient shell of <flavor> in cwd.
    #[arg(add = ArgValueCompleter::new(complete_top_level))]
    pub flavor: Option<String>,
}

#[derive(Subcommand)]
pub enum Cmd {
    Init {
        #[arg(short = 'p', long = "private")]
        private: bool,
        #[arg(add = ArgValueCompleter::new(complete_flavor))]
        flavor: String,
    },
    Shell,
    Run,
    Stop,
    Build {
        #[arg(add = ArgValueCompleter::new(complete_flavor_or_all))]
        flavor: Option<String>,
    },
    Rebuild {
        #[arg(add = ArgValueCompleter::new(complete_flavor_or_all))]
        flavor: Option<String>,
    },
    Clean {
        #[arg(add = ArgValueCompleter::new(complete_flavor))]
        flavor: Option<String>,
    },
    Purge {
        #[arg(add = ArgValueCompleter::new(complete_flavor))]
        flavor: Option<String>,
    },
    List,
    #[command(alias = "ps", alias = "ls-sessions")]
    Sessions,
    #[command(alias = "ports")]
    Port {
        #[command(subcommand)]
        action: Option<PortCmd>,
    },
    Env {
        #[command(subcommand)]
        action: Option<EnvCmd>,
    },
    Start {
        #[command(subcommand)]
        action: Option<StartCmd>,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, hide = true)]
        rest: Vec<String>,
    },
    Scan {
        #[arg(value_parser = ["fs", "filesystem", "image"], default_value = "fs")]
        target: String,
    },
    Net {
        #[command(subcommand)]
        action: Option<NetCmd>,
    },
    Ssh {
        #[command(subcommand)]
        action: Option<SshCmd>,
    },
    #[command(alias = "services")]
    Service {
        #[command(subcommand)]
        action: Option<ServiceCmd>,
    },
    Completions {
        shell: clap_complete::Shell,
    },
    Claude {
        #[command(subcommand)]
        action: Option<ClaudeCmd>,
        #[arg(short = 'm', long = "mount", value_name = "PATH")]
        mounts: Vec<String>,
        #[arg(short = 'p', long = "profile", value_name = "NAME",
              add = ArgValueCompleter::new(complete_claude_profile))]
        profile: Option<String>,
        #[arg(short = 's', long = "safe")]
        safe: bool,
        #[arg(long = "no-rc")]
        no_rc: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum ClaudeCmd {
    #[command(alias = "bash")]
    Shell,
    Build,
    Rebuild,
    Profile {
        #[command(subcommand)]
        action: Option<ProfileCmd>,
    },
}

#[derive(Subcommand)]
pub enum ProfileCmd {
    #[command(alias = "ls")]
    List,
    Add {
        name: String,
    },
    #[command(alias = "remove", alias = "del", alias = "delete")]
    Rm {
        #[arg(add = ArgValueCompleter::new(complete_claude_profile))]
        name: String,
    },
    Current,
}

#[derive(Subcommand)]
pub enum PortCmd {
    #[command(alias = "ls")]
    List,
    Add {
        port: String,
    },
    #[command(alias = "remove", alias = "del", alias = "delete")]
    Rm {
        #[arg(add = ArgValueCompleter::new(complete_configured_port))]
        port: String,
    },
}

#[derive(Subcommand)]
pub enum EnvCmd {
    #[command(alias = "ls")]
    List,
    Set {
        spec: String,
        value: Option<String>,
    },
    #[command(alias = "rm", alias = "remove", alias = "del")]
    Unset {
        #[arg(add = ArgValueCompleter::new(complete_env_key))]
        key: String,
    },
}

#[derive(Subcommand)]
pub enum StartCmd {
    Show,
    Set {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        cmd: Vec<String>,
    },
    #[command(alias = "rm", alias = "remove", alias = "unset")]
    Clear,
}

#[derive(Subcommand)]
pub enum NetCmd {
    Vpn {
        #[command(subcommand)]
        action: Option<VpnCmd>,
    },
    Tailscale {
        #[command(subcommand)]
        action: Option<TailscaleCmd>,
    },
}

#[derive(Subcommand)]
pub enum TailscaleCmd {
    #[command(alias = "enable")]
    On {
        #[arg(add = ArgValueCompleter::new(complete_tailscale_profile))]
        name: Option<String>,
    },
    #[command(alias = "disable")]
    Off,
    Status,
    Auth {
        name: Option<String>,
    },
    #[command(alias = "ls")]
    List,
    #[command(alias = "remove", alias = "del", alias = "delete")]
    Rm {
        #[arg(add = ArgValueCompleter::new(complete_tailscale_profile))]
        name: String,
    },
}

#[derive(Subcommand)]
pub enum VpnCmd {
    Status,
    Use {
        #[arg(add = ArgValueCompleter::new(complete_vpn_name))]
        spec: String,
    },
    Auth,
    Inline {
        target: Option<std::path::PathBuf>,
    },
    #[command(alias = "unuse", alias = "clear")]
    Off,
}

#[derive(Subcommand)]
pub enum SshCmd {
    #[command(alias = "enable")]
    On,
    #[command(alias = "disable")]
    Off,
    Status,
}

#[derive(Subcommand)]
pub enum ServiceCmd {
    #[command(alias = "ls")]
    List,
    Add {
        #[arg(add = ArgValueCompleter::new(complete_service))]
        name: String,
    },
    #[command(alias = "remove", alias = "del", alias = "delete")]
    Rm {
        #[arg(add = ArgValueCompleter::new(complete_configured_service))]
        name: String,
    },
}
