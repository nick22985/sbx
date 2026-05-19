use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::{
    Claude, Config, FlavorConfig, GlobalConfig, HostProxy, Network, Services, Tunnel, TunnelRight,
};
use crate::project::{private_sbx_dir, shared_root};
use crate::util::{config_dir, config_lines, die, flavors_dir, log};

const LEGACY_PROJECT_FILES: &[&str] = &[
    "flavor",
    "name",
    "port-offset",
    "ports",
    "start",
    "mounts",
    "caches",
    "ssh",
    "docker",
    "gui",
    "claude-profile",
    "network",
    "hostname",
    "public",
    "tunnels",
    "services",
    "host-proxy",
];

pub fn run(cwd: &Path) {
    let mut total_migrated = 0usize;
    total_migrated += migrate_global();
    total_migrated += migrate_all_flavors();
    total_migrated += migrate_project(cwd);

    if total_migrated == 0 {
        log("nothing to migrate.");
    }
}

fn migrate_global() -> usize {
    let dir = config_dir();
    if !dir.is_dir() {
        return 0;
    }
    let cfg_path = dir.join("config.toml");
    if cfg_path.exists() {
        return 0;
    }
    let mounts_path = dir.join("mounts");
    let caches_path = dir.join("caches");
    let mut found: Vec<&'static str> = Vec::new();
    let mut cfg = GlobalConfig::default();
    if mounts_path.is_file() {
        cfg.mounts = read_lines(&mounts_path);
        found.push("mounts");
    }
    if caches_path.is_file() {
        cfg.caches = read_lines(&caches_path);
        found.push("caches");
    }
    if found.is_empty() {
        return 0;
    }
    let written = cfg
        .save()
        .unwrap_or_else(|e| die(format!("write global config.toml: {e}")));
    log(format!("wrote {}", written.display()));
    move_to_legacy(&dir, &found);
    found.len()
}

fn migrate_all_flavors() -> usize {
    let mut total = relocate_top_level_flavor_dirs();

    let flavors = flavors_dir();
    let Ok(entries) = fs::read_dir(&flavors) else {
        return total;
    };
    for e in entries.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let Some(name) = e.file_name().to_str().map(|s| s.to_string()) else {
            continue;
        };
        if !e.path().join("Dockerfile").is_file() {
            continue;
        }
        total += migrate_flavor(&name, &e.path());
    }
    total
}

fn relocate_top_level_flavor_dirs() -> usize {
    let cfg = config_dir();
    let flavors = flavors_dir();
    let Ok(entries) = fs::read_dir(&cfg) else {
        return 0;
    };
    let mut to_move: Vec<String> = Vec::new();
    for e in entries.flatten() {
        let Ok(ft) = e.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let Some(name) = e.file_name().to_str().map(|s| s.to_string()) else {
            continue;
        };
        if name == "flavors" || name == "legacy" {
            continue;
        }
        if !e.path().join("Dockerfile").is_file() {
            continue;
        }
        to_move.push(name);
    }
    if to_move.is_empty() {
        return 0;
    }
    if let Err(e) = fs::create_dir_all(&flavors) {
        die(format!("mkdir {}: {e}", flavors.display()));
    }
    for name in &to_move {
        let src = cfg.join(name);
        let dst = flavors.join(name);
        if let Err(e) = fs::rename(&src, &dst) {
            die(format!("move {} -> {}: {e}", src.display(), dst.display()));
        }
        log(format!("moved flavor {} -> {}", name, dst.display()));
    }
    to_move.len()
}

fn migrate_flavor(flavor: &str, dir: &Path) -> usize {
    let cfg_path = dir.join("config.toml");
    if cfg_path.exists() {
        return 0;
    }
    let mounts_path = dir.join("mounts");
    let caches_path = dir.join("caches");
    let mut found: Vec<&'static str> = Vec::new();
    let mut cfg = FlavorConfig::default();
    if mounts_path.is_file() {
        cfg.mounts = read_lines(&mounts_path);
        found.push("mounts");
    }
    if caches_path.is_file() {
        cfg.caches = read_lines(&caches_path);
        found.push("caches");
    }
    if found.is_empty() {
        return 0;
    }
    let written = cfg
        .save(flavor)
        .unwrap_or_else(|e| die(format!("write flavor config.toml: {e}")));
    log(format!("wrote {}", written.display()));
    move_to_legacy(dir, &found);
    found.len()
}

fn migrate_project(cwd: &Path) -> usize {
    let mut total = 0;
    let mut seen: Vec<PathBuf> = Vec::new();
    for dir in project_candidate_dirs(cwd) {
        if seen.iter().any(|p| p == &dir) {
            continue;
        }
        seen.push(dir.clone());
        total += migrate_project_dir(&dir);
    }
    total
}

fn project_candidate_dirs(cwd: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    out.push(cwd.join(".sbx"));
    if let Some(shared) = shared_root(cwd) {
        out.push(shared.join(".sbx"));
    }
    if let Some(priv_dir) = private_sbx_dir(cwd) {
        out.push(priv_dir);
    }
    out
}

fn migrate_project_dir(dir: &Path) -> usize {
    if !dir.is_dir() {
        return 0;
    }
    let cfg_path = dir.join("config.toml");
    if cfg_path.exists() {
        return 0;
    }

    let mut cfg = Config::default();
    let mut found: Vec<&'static str> = Vec::new();

    for name in LEGACY_PROJECT_FILES {
        let p = dir.join(name);
        if !p.exists() {
            continue;
        }
        match *name {
            "flavor" => cfg.flavor = read_trimmed(&p),
            "name" => cfg.name = read_trimmed(&p),
            "port-offset" => {
                if let Some(s) = read_trimmed(&p)
                    && let Ok(n) = s.parse::<u16>()
                {
                    cfg.port_offset = Some(n);
                }
            }
            "ports" => cfg.ports = read_ports(&p),
            "start" => cfg.start = read_body_nonempty(&p),
            "mounts" => cfg.mounts = read_lines(&p),
            "caches" => cfg.caches = read_lines(&p),
            "ssh" => cfg.ssh = true,
            "docker" => cfg.docker = true,
            "gui" => cfg.gui = true,
            "claude-profile" => {
                cfg.claude = Claude {
                    profile: read_trimmed(&p),
                };
            }
            "network" => cfg.network = read_network(&p),
            "hostname" => cfg.hostname = read_name_port_map(&p),
            "public" => cfg.public = read_name_port_map(&p),
            "tunnels" => cfg.tunnels = read_tunnels(&p),
            "services" => {
                cfg.services = Services {
                    enabled: read_lines(&p),
                };
            }
            "host-proxy" => {
                cfg.host_proxy = HostProxy {
                    enabled: true,
                    allow: read_lines(&p),
                };
            }
            _ => {}
        }
        found.push(name);
    }

    if found.is_empty() {
        return 0;
    }

    let written = cfg
        .save_to_dir(dir)
        .unwrap_or_else(|e| die(format!("write config.toml: {e}")));
    log(format!("wrote {}", written.display()));
    move_to_legacy(dir, &found);
    found.len()
}

fn move_to_legacy(dir: &Path, names: &[&str]) {
    let legacy_dir: PathBuf = dir.join("legacy");
    if let Err(e) = fs::create_dir_all(&legacy_dir) {
        die(format!("mkdir {}: {e}", legacy_dir.display()));
    }
    for name in names {
        let src = dir.join(name);
        let dst = legacy_dir.join(name);
        if let Err(e) = fs::rename(&src, &dst) {
            die(format!("move {} -> {}: {e}", src.display(), dst.display()));
        }
    }
    log(format!(
        "moved {} legacy file(s) to {}",
        names.len(),
        legacy_dir.display()
    ));
}

fn read_trimmed(p: &Path) -> Option<String> {
    let s = fs::read_to_string(p).ok()?.trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

fn read_body_nonempty(p: &Path) -> Option<String> {
    let s = fs::read_to_string(p).ok()?;
    let body = s.trim_end_matches('\n').to_string();
    if body.is_empty() { None } else { Some(body) }
}

fn read_lines(p: &Path) -> Vec<String> {
    let Ok(s) = fs::read_to_string(p) else {
        return Vec::new();
    };
    config_lines(&s).map(|l| l.to_string()).collect()
}

fn read_ports(p: &Path) -> Vec<u16> {
    let Ok(s) = fs::read_to_string(p) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in config_lines(&s) {
        let cleaned: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        if let Ok(n) = cleaned.parse::<u16>() {
            out.push(n);
        }
    }
    out
}

fn read_network(p: &Path) -> Network {
    let mut net = Network::default();
    let Ok(s) = fs::read_to_string(p) else {
        return net;
    };
    for line in config_lines(&s) {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim();
        if val.is_empty() {
            continue;
        }
        match key {
            "vpn" => net.vpn = Some(val.to_string()),
            "tailscale" => net.tailscale = Some(val.to_string()),
            _ => {}
        }
    }
    net
}

fn read_name_port_map(p: &Path) -> BTreeMap<String, u16> {
    let mut out = BTreeMap::new();
    let Ok(s) = fs::read_to_string(p) else {
        return out;
    };
    for line in config_lines(&s) {
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        let host = lhs.trim().to_string();
        if host.is_empty() {
            continue;
        }
        let Ok(port) = rhs.trim().parse::<u16>() else {
            continue;
        };
        out.insert(host, port);
    }
    out
}

fn read_tunnels(p: &Path) -> Vec<Tunnel> {
    let Ok(s) = fs::read_to_string(p) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in config_lines(&s) {
        let Some((dir_part, rest)) = line.split_once(':') else {
            continue;
        };
        let Some((lhs, rhs)) = rest.split_once('=') else {
            continue;
        };
        let Ok(left) = lhs.trim().parse::<u16>() else {
            continue;
        };
        let dir = dir_part.trim();
        let right_raw = rhs.trim();
        if dir.is_empty() || right_raw.is_empty() {
            continue;
        }
        let right = match right_raw.parse::<u16>() {
            Ok(p) => TunnelRight::Port(p),
            Err(_) => TunnelRight::Address(right_raw.to_string()),
        };
        out.push(Tunnel {
            dir: dir.to_string(),
            left,
            right,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::set_test_paths;

    fn tmp_dir(label: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("sbx-mg-{label}-{pid}-{nanos}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write(p: &Path, s: &str) {
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, s).unwrap();
    }

    #[test]
    fn migrate_global_writes_config_and_moves_legacy() {
        let cfg = tmp_dir("mg-glob-cfg");
        let home = tmp_dir("mg-glob-home");
        let _g = set_test_paths(cfg.clone(), home);
        fs::create_dir_all(&cfg).unwrap();
        write(&cfg.join("mounts"), "~/a\n# comment\n~/b\n");
        write(&cfg.join("caches"), ".cache/x\n.cache/y\n");

        let count = migrate_global();
        assert_eq!(count, 2);
        let written = fs::read_to_string(cfg.join("config.toml")).unwrap();
        assert!(written.contains("\"~/a\""));
        assert!(written.contains("\"~/b\""));
        assert!(written.contains("\".cache/x\""));
        assert!(cfg.join("legacy/mounts").is_file());
        assert!(cfg.join("legacy/caches").is_file());
        assert!(!cfg.join("mounts").exists());
        assert!(!cfg.join("caches").exists());
    }

    #[test]
    fn migrate_global_idempotent_when_config_toml_exists() {
        let cfg = tmp_dir("mg-glob-idem-cfg");
        let home = tmp_dir("mg-glob-idem-home");
        let _g = set_test_paths(cfg.clone(), home);
        fs::create_dir_all(&cfg).unwrap();
        write(&cfg.join("config.toml"), "mounts = []\n");
        write(&cfg.join("mounts"), "~/a\n");

        let count = migrate_global();
        assert_eq!(count, 0);
        assert!(cfg.join("mounts").exists(), "legacy file should not move when config.toml present");
    }

    #[test]
    fn migrate_flavor_writes_config_and_moves_legacy() {
        let cfg = tmp_dir("mg-flav-cfg");
        let home = tmp_dir("mg-flav-home");
        let _g = set_test_paths(cfg.clone(), home);
        let flav_dir = cfg.join("flavors/npm");
        write(&flav_dir.join("Dockerfile"), "FROM alpine\n");
        write(&flav_dir.join("mounts"), "~/.npm\n");
        write(&flav_dir.join("caches"), ".cache/npm\n");

        let count = migrate_all_flavors();
        assert_eq!(count, 2);
        let written = fs::read_to_string(flav_dir.join("config.toml")).unwrap();
        assert!(written.contains("\"~/.npm\""));
        assert!(written.contains("\".cache/npm\""));
        assert!(flav_dir.join("legacy/mounts").is_file());
    }

    #[test]
    fn relocate_top_level_flavor_dirs_moves_into_flavors_subdir() {
        let cfg = tmp_dir("mg-reloc-cfg");
        let home = tmp_dir("mg-reloc-home");
        let _g = set_test_paths(cfg.clone(), home);
        write(&cfg.join("npm/Dockerfile"), "FROM alpine\n");
        write(&cfg.join("npm/mounts"), "~/.npm\n");
        // Existing flavors/ should not be touched.
        write(&cfg.join("flavors/.keep"), "");

        let count = relocate_top_level_flavor_dirs();
        assert_eq!(count, 1);
        assert!(cfg.join("flavors/npm/Dockerfile").is_file());
        assert!(cfg.join("flavors/npm/mounts").is_file());
        assert!(!cfg.join("npm").exists());
    }

    #[test]
    fn relocate_skips_flavors_and_legacy_dirs() {
        let cfg = tmp_dir("mg-skip-cfg");
        let home = tmp_dir("mg-skip-home");
        let _g = set_test_paths(cfg.clone(), home);
        write(&cfg.join("flavors/.keep"), "");
        write(&cfg.join("legacy/.keep"), "");
        write(&cfg.join("legacy/Dockerfile"), "");

        let count = relocate_top_level_flavor_dirs();
        assert_eq!(count, 0);
        assert!(cfg.join("legacy/Dockerfile").is_file(), "legacy must not move");
    }

    #[test]
    fn migrate_project_dir_handles_all_legacy_files() {
        let dir = tmp_dir("mg-proj").join(".sbx");
        fs::create_dir_all(&dir).unwrap();
        write(&dir.join("flavor"), "npm\n");
        write(&dir.join("ports"), "8080\n3000\n");
        write(&dir.join("start"), "npm run dev\n");
        write(&dir.join("ssh"), "");
        write(&dir.join("mounts"), "~/proj-mount\n");
        write(&dir.join("network"), "vpn=/tmp/x.ovpn\n");
        write(&dir.join("hostname"), "abi.local=8080\n");
        write(&dir.join("tunnels"), "out:33201=33201\nin:33301=db:5432\n");

        let count = migrate_project_dir(&dir);
        assert_eq!(count, 8);
        let cfg: Config = toml::from_str(
            &fs::read_to_string(dir.join("config.toml")).unwrap(),
        )
        .unwrap();
        assert_eq!(cfg.flavor.as_deref(), Some("npm"));
        assert_eq!(cfg.ports, vec![8080, 3000]);
        assert_eq!(cfg.start.as_deref(), Some("npm run dev"));
        assert!(cfg.ssh);
        assert_eq!(cfg.mounts, vec!["~/proj-mount"]);
        assert_eq!(cfg.network.vpn.as_deref(), Some("/tmp/x.ovpn"));
        assert_eq!(cfg.hostname.get("abi.local"), Some(&8080));
        assert_eq!(cfg.tunnels.len(), 2);
        match &cfg.tunnels[1].right {
            TunnelRight::Address(s) => assert_eq!(s, "db:5432"),
            _ => panic!("expected Address variant"),
        }
        assert!(dir.join("legacy/flavor").is_file());
        assert!(!dir.join("flavor").exists());
    }

    #[test]
    fn migrate_project_dir_no_legacy_returns_zero() {
        let dir = tmp_dir("mg-empty").join(".sbx");
        fs::create_dir_all(&dir).unwrap();
        let count = migrate_project_dir(&dir);
        assert_eq!(count, 0);
        assert!(!dir.join("config.toml").exists());
        assert!(!dir.join("legacy").exists());
    }

    #[test]
    fn migrate_project_dir_skips_when_config_toml_already_exists() {
        let dir = tmp_dir("mg-exists").join(".sbx");
        fs::create_dir_all(&dir).unwrap();
        write(&dir.join("config.toml"), "flavor = \"existing\"\n");
        write(&dir.join("flavor"), "old\n");
        let count = migrate_project_dir(&dir);
        assert_eq!(count, 0);
        assert!(dir.join("flavor").is_file(), "legacy must not move when config.toml present");
    }

    #[test]
    fn read_ports_parses_and_ignores_garbage() {
        let p = tmp_dir("read-ports").join("ports");
        write(&p, "8080\n# comment\n3000\nnot a port\n  9000  \n");
        let ports = read_ports(&p);
        assert_eq!(ports, vec![8080, 3000, 9000]);
    }

    #[test]
    fn read_tunnels_parses_port_and_address_variants() {
        let p = tmp_dir("read-tunnels").join("tunnels");
        write(&p, "out:33201=33201\nin:33301=db:5432\nbad line\n");
        let tunnels = read_tunnels(&p);
        assert_eq!(tunnels.len(), 2);
        assert_eq!(tunnels[0].dir, "out");
        assert_eq!(tunnels[0].left, 33201);
        match &tunnels[0].right {
            TunnelRight::Port(p) => assert_eq!(*p, 33201),
            _ => panic!("expected Port variant"),
        }
        match &tunnels[1].right {
            TunnelRight::Address(s) => assert_eq!(s, "db:5432"),
            _ => panic!("expected Address variant"),
        }
    }

    #[test]
    fn read_network_parses_vpn_and_tailscale() {
        let p = tmp_dir("read-net").join("network");
        write(&p, "vpn=/tmp/x.ovpn\ntailscale=mfdc\nunknown=ignored\n");
        let net = read_network(&p);
        assert_eq!(net.vpn.as_deref(), Some("/tmp/x.ovpn"));
        assert_eq!(net.tailscale.as_deref(), Some("mfdc"));
    }

    #[test]
    fn read_name_port_map_parses_kv() {
        let p = tmp_dir("read-map").join("hostname");
        write(&p, "abi.local=8080\nother.local=3000\nmalformed\n");
        let m = read_name_port_map(&p);
        assert_eq!(m.get("abi.local"), Some(&8080));
        assert_eq!(m.get("other.local"), Some(&3000));
        assert_eq!(m.len(), 2);
    }
}

