use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::project::{private_sbx_dir, shared_root};
use crate::util::{config_dir, flavors_dir};

pub const CONFIG_FILENAME: &str = "config.toml";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flavor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port_offset: Option<u16>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ports: Vec<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caches: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ssh: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub docker: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gui: bool,

    #[serde(default, skip_serializing_if = "Claude::is_empty")]
    pub claude: Claude,
    #[serde(default, skip_serializing_if = "Network::is_empty")]
    pub network: Network,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hostname: BTreeMap<String, u16>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub public: BTreeMap<String, u16>,
    #[serde(default, rename = "tunnel", skip_serializing_if = "Vec::is_empty")]
    pub tunnels: Vec<Tunnel>,
    #[serde(default, rename = "socks", skip_serializing_if = "Vec::is_empty")]
    pub socks: Vec<Socks>,
    #[serde(default, skip_serializing_if = "Services::is_empty")]
    pub services: Services,
    #[serde(
        default,
        rename = "host_proxy",
        skip_serializing_if = "HostProxy::is_empty"
    )]
    pub host_proxy: HostProxy,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Claude {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

impl Claude {
    fn is_empty(&self) -> bool {
        self.profile.is_none()
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Network {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vpn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tailscale: Option<String>,
}

impl Network {
    fn is_empty(&self) -> bool {
        self.vpn.is_none() && self.tailscale.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Tunnel {
    pub dir: String,
    pub left: u16,
    pub right: TunnelRight,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TunnelRight {
    Port(u16),
    Address(String),
}

impl TunnelRight {
    pub fn as_string(&self) -> String {
        match self {
            TunnelRight::Port(p) => p.to_string(),
            TunnelRight::Address(s) => s.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Socks {
    pub port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pass: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Services {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub enabled: Vec<String>,
}

impl Services {
    fn is_empty(&self) -> bool {
        self.enabled.is_empty()
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct HostProxy {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
}

impl HostProxy {
    fn is_empty(&self) -> bool {
        !self.enabled && self.allow.is_empty()
    }
}

impl Config {
    pub fn path(root: &Path) -> PathBuf {
        let local = root.join(".sbx").join(CONFIG_FILENAME);
        if local.exists() {
            return local;
        }
        if let Some(shared) = shared_root(root) {
            let p = shared.join(".sbx").join(CONFIG_FILENAME);
            if p.exists() {
                return p;
            }
        }
        if let Some(priv_dir) = private_sbx_dir(root) {
            let p = priv_dir.join(CONFIG_FILENAME);
            if p.exists() {
                return p;
            }
        }
        local
    }

    pub fn exists_for(root: &Path) -> bool {
        !Self::layered_paths(root).is_empty()
    }

    pub fn load(root: &Path) -> io::Result<Self> {
        let p = Self::path(root);
        let raw = fs::read_to_string(&p)?;
        toml::from_str(&raw).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, format!("{}: {e}", p.display()))
        })
    }

    fn load_single(root: &Path) -> Self {
        Self::load(root).unwrap_or_default()
    }

    /// Layered load: merges private → shared → local config.tomls.
    /// Scalar fields take the last non-None (so local overrides private);
    /// bool fields are OR'd; vec fields are concatenated and deduped
    /// preserving first occurrence; map fields are merged with later keys
    /// winning. This matches the pre-TOML behavior where each setting
    /// could be sourced independently from any of the three locations.
    pub fn load_or_default(root: &Path) -> Self {
        let mut out = Self::default();
        for path in Self::layered_paths(root) {
            let Ok(raw) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(cfg) = toml::from_str::<Self>(&raw) else {
                continue;
            };
            out.merge(cfg);
        }
        out
    }

    fn layered_paths(root: &Path) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        if let Some(priv_dir) = private_sbx_dir(root) {
            let p = priv_dir.join(CONFIG_FILENAME);
            if p.is_file() {
                out.push(p);
            }
        }
        if let Some(shared) = shared_root(root) {
            let p = shared.join(".sbx").join(CONFIG_FILENAME);
            if p.is_file() && !out.contains(&p) {
                out.push(p);
            }
        }
        let local = root.join(".sbx").join(CONFIG_FILENAME);
        if local.is_file() && !out.contains(&local) {
            out.push(local);
        }
        out
    }

    fn merge(&mut self, other: Self) {
        if other.flavor.is_some() {
            self.flavor = other.flavor;
        }
        if other.name.is_some() {
            self.name = other.name;
        }
        if other.port_offset.is_some() {
            self.port_offset = other.port_offset;
        }
        if other.start.is_some() {
            self.start = other.start;
        }
        self.ssh |= other.ssh;
        self.docker |= other.docker;
        self.gui |= other.gui;
        extend_dedup(&mut self.ports, other.ports);
        extend_dedup(&mut self.mounts, other.mounts);
        extend_dedup(&mut self.caches, other.caches);
        extend_dedup_tunnels(&mut self.tunnels, other.tunnels);
        extend_dedup_socks(&mut self.socks, other.socks);
        for (k, v) in other.hostname {
            self.hostname.insert(k, v);
        }
        for (k, v) in other.public {
            self.public.insert(k, v);
        }
        if other.claude.profile.is_some() {
            self.claude.profile = other.claude.profile;
        }
        if other.network.vpn.is_some() {
            self.network.vpn = other.network.vpn;
        }
        if other.network.tailscale.is_some() {
            self.network.tailscale = other.network.tailscale;
        }
        extend_dedup(&mut self.services.enabled, other.services.enabled);
        self.host_proxy.enabled |= other.host_proxy.enabled;
        extend_dedup(&mut self.host_proxy.allow, other.host_proxy.allow);
    }

    pub fn save(&self, root: &Path) -> io::Result<PathBuf> {
        self.save_to_file(&Self::path(root))
    }

    pub fn save_to_dir(&self, sbx_dir: &Path) -> io::Result<PathBuf> {
        self.save_to_file(&sbx_dir.join(CONFIG_FILENAME))
    }

    fn save_to_file(&self, path: &Path) -> io::Result<PathBuf> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let serialized = toml::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("serialize: {e}")))?;
        fs::write(path, serialized)?;
        Ok(path.to_path_buf())
    }

    pub fn edit<F: FnOnce(&mut Config)>(root: &Path, f: F) -> io::Result<PathBuf> {
        let mut cfg = Self::load_single(root);
        f(&mut cfg);
        cfg.save(root)
    }
}

fn extend_dedup<T: Eq + Clone>(dst: &mut Vec<T>, src: Vec<T>) {
    for item in src {
        if !dst.iter().any(|x| x == &item) {
            dst.push(item);
        }
    }
}

fn extend_dedup_tunnels(dst: &mut Vec<Tunnel>, src: Vec<Tunnel>) {
    let key = |t: &Tunnel| (t.dir.clone(), t.left, t.right.as_string());
    let mut seen: Vec<(String, u16, String)> = dst.iter().map(key).collect();
    for t in src {
        let k = key(&t);
        if !seen.contains(&k) {
            seen.push(k);
            dst.push(t);
        }
    }
}

fn extend_dedup_socks(dst: &mut Vec<Socks>, src: Vec<Socks>) {
    let mut seen: Vec<u16> = dst.iter().map(|s| s.port).collect();
    for s in src {
        if !seen.contains(&s.port) {
            seen.push(s.port);
            dst.push(s);
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct GlobalConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caches: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_home: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_bare_repo: bool,
}

impl GlobalConfig {
    pub fn path() -> PathBuf {
        config_dir().join(CONFIG_FILENAME)
    }

    pub fn load_or_default() -> Self {
        let p = Self::path();
        match fs::read_to_string(&p) {
            Ok(raw) => toml::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> io::Result<PathBuf> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let serialized = toml::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("serialize: {e}")))?;
        fs::write(&path, serialized)?;
        Ok(path)
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct FlavorConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub caches: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_bare_repo: bool,
}

impl FlavorConfig {
    pub fn path(flavor: &str) -> PathBuf {
        flavors_dir().join(flavor).join(CONFIG_FILENAME)
    }

    pub fn load_or_default(flavor: &str) -> Self {
        let p = Self::path(flavor);
        match fs::read_to_string(&p) {
            Ok(raw) => toml::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, flavor: &str) -> io::Result<PathBuf> {
        let path = Self::path(flavor);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let serialized = toml::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("serialize: {e}")))?;
        fs::write(&path, serialized)?;
        Ok(path)
    }
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
        let path = std::env::temp_dir().join(format!("sbx-cfg-{label}-{pid}-{nanos}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn round_trip<T: Serialize + serde::de::DeserializeOwned>(v: &T) -> String {
        let s = toml::to_string_pretty(v).unwrap();
        let parsed: T = toml::from_str(&s).unwrap();
        toml::to_string_pretty(&parsed).unwrap()
    }

    #[test]
    fn flavor_config_path_uses_flavors_subdir() {
        let cfg = tmp_dir("flavor-path-cfg");
        let home = tmp_dir("flavor-path-home");
        let _g = set_test_paths(cfg.clone(), home);
        let p = FlavorConfig::path("nvim");
        assert_eq!(p, cfg.join("flavors").join("nvim").join("config.toml"));
    }

    #[test]
    fn flavor_config_save_and_load_round_trip() {
        let cfg = tmp_dir("flavor-save-cfg");
        let home = tmp_dir("flavor-save-home");
        let _g = set_test_paths(cfg.clone(), home);
        let original = FlavorConfig {
            mounts: vec!["~/.config/nvim".into()],
            caches: vec![".local/share/nvim".into(), ".cache/nvim".into()],
            start: Some("nvim .".into()),
            allow_bare_repo: false,
        };
        let written = original.save("nvim").unwrap();
        assert_eq!(written, cfg.join("flavors/nvim/config.toml"));
        let loaded = FlavorConfig::load_or_default("nvim");
        assert_eq!(loaded.mounts, original.mounts);
        assert_eq!(loaded.caches, original.caches);
        assert_eq!(loaded.start, original.start);
    }

    #[test]
    fn flavor_config_skip_empty_serializes_to_nothing() {
        let empty = FlavorConfig::default();
        let s = toml::to_string_pretty(&empty).unwrap();
        assert!(
            s.trim().is_empty(),
            "expected empty serialization, got {s:?}"
        );
    }

    #[test]
    fn global_config_round_trip() {
        let cfg = GlobalConfig {
            mounts: vec!["~/.cache/shared".into()],
            caches: vec![".cache/global".into()],
            container_home: None,
            allow_bare_repo: false,
        };
        let a = round_trip(&cfg);
        let b = round_trip(&cfg);
        assert_eq!(a, b);
        let parsed: GlobalConfig = toml::from_str(&a).unwrap();
        assert_eq!(parsed.mounts, cfg.mounts);
        assert_eq!(parsed.caches, cfg.caches);
    }

    #[test]
    fn config_round_trip_full() {
        let mut hostname = BTreeMap::new();
        hostname.insert("abi.local".to_string(), 8080u16);
        let mut public = BTreeMap::new();
        public.insert("abi.example".to_string(), 8080u16);

        let cfg = Config {
            flavor: Some("npm".into()),
            name: Some("exp1".into()),
            port_offset: Some(3),
            ports: vec![8080, 3000],
            start: Some("npm run dev".into()),
            mounts: vec!["~/.npm".into()],
            caches: vec![".cache/npm".into()],
            ssh: true,
            docker: false,
            gui: true,
            claude: Claude {
                profile: Some("work".into()),
            },
            network: Network {
                vpn: Some("/tmp/x.ovpn".into()),
                tailscale: None,
            },
            hostname,
            public,
            tunnels: vec![
                Tunnel {
                    dir: "out".into(),
                    left: 33201,
                    right: TunnelRight::Port(33201),
                },
                Tunnel {
                    dir: "in".into(),
                    left: 33202,
                    right: TunnelRight::Address("db:5432".into()),
                },
            ],
            socks: vec![Socks {
                port: 1080,
                name: Some("mongo".into()),
                user: None,
                pass: None,
            }],
            services: Services {
                enabled: vec!["postgres".into()],
            },
            host_proxy: HostProxy {
                enabled: true,
                allow: vec!["github.com".into()],
            },
        };

        let s1 = toml::to_string_pretty(&cfg).unwrap();
        let parsed: Config = toml::from_str(&s1).unwrap();
        let s2 = toml::to_string_pretty(&parsed).unwrap();
        assert_eq!(s1, s2, "round-trip should be stable");

        assert_eq!(parsed.flavor.as_deref(), Some("npm"));
        assert_eq!(parsed.ports, vec![8080, 3000]);
        assert!(parsed.ssh);
        assert!(!parsed.docker);
        assert!(parsed.gui);
        assert_eq!(parsed.tunnels.len(), 2);
        assert_eq!(parsed.tunnels[0].left, 33201);
        match &parsed.tunnels[1].right {
            TunnelRight::Address(s) => assert_eq!(s, "db:5432"),
            TunnelRight::Port(_) => panic!("expected Address variant"),
        }
        assert_eq!(parsed.services.enabled, vec!["postgres"]);
        assert!(parsed.host_proxy.enabled);
        assert_eq!(parsed.network.vpn.as_deref(), Some("/tmp/x.ovpn"));
        assert_eq!(parsed.claude.profile.as_deref(), Some("work"));
    }

    #[test]
    fn config_default_serializes_to_empty() {
        let cfg = Config::default();
        let s = toml::to_string_pretty(&cfg).unwrap();
        assert!(
            s.trim().is_empty(),
            "expected empty default serialization, got {s:?}"
        );
    }

    #[test]
    fn merge_scalar_local_overrides_private() {
        let mut a = Config {
            flavor: Some("private-flavor".into()),
            start: Some("private start".into()),
            ..Default::default()
        };
        let b = Config {
            flavor: Some("local-flavor".into()),
            start: None,
            ..Default::default()
        };
        a.merge(b);
        assert_eq!(a.flavor.as_deref(), Some("local-flavor"));
        assert_eq!(a.start.as_deref(), Some("private start"));
    }

    #[test]
    fn merge_vec_concat_dedupes() {
        let mut a = Config {
            mounts: vec!["~/a".into(), "~/b".into()],
            ports: vec![8000, 8001],
            ..Default::default()
        };
        let b = Config {
            mounts: vec!["~/b".into(), "~/c".into()],
            ports: vec![8001, 8002],
            ..Default::default()
        };
        a.merge(b);
        assert_eq!(a.mounts, vec!["~/a", "~/b", "~/c"]);
        assert_eq!(a.ports, vec![8000, 8001, 8002]);
    }

    #[test]
    fn merge_bool_ors_flags() {
        let mut a = Config {
            ssh: true,
            docker: false,
            gui: false,
            ..Default::default()
        };
        let b = Config {
            ssh: false,
            docker: true,
            gui: false,
            ..Default::default()
        };
        a.merge(b);
        assert!(a.ssh);
        assert!(a.docker);
        assert!(!a.gui);
    }

    #[test]
    fn merge_map_local_overrides() {
        let mut a_host = BTreeMap::new();
        a_host.insert("abi.local".into(), 8000u16);
        let mut a = Config {
            hostname: a_host,
            ..Default::default()
        };
        let mut b_host = BTreeMap::new();
        b_host.insert("abi.local".into(), 9000u16);
        b_host.insert("other.local".into(), 7000u16);
        let b = Config {
            hostname: b_host,
            ..Default::default()
        };
        a.merge(b);
        assert_eq!(a.hostname.get("abi.local"), Some(&9000));
        assert_eq!(a.hostname.get("other.local"), Some(&7000));
    }

    #[test]
    fn merge_tunnels_dedupes_by_dir_left_right() {
        let mut a = Config {
            tunnels: vec![Tunnel {
                dir: "out".into(),
                left: 33201,
                right: TunnelRight::Port(33201),
            }],
            ..Default::default()
        };
        let b = Config {
            tunnels: vec![
                Tunnel {
                    dir: "out".into(),
                    left: 33201,
                    right: TunnelRight::Port(33201),
                },
                Tunnel {
                    dir: "in".into(),
                    left: 33301,
                    right: TunnelRight::Address("db:5432".into()),
                },
            ],
            ..Default::default()
        };
        a.merge(b);
        assert_eq!(a.tunnels.len(), 2);
        assert_eq!(a.tunnels[1].dir, "in");
    }

    #[test]
    fn load_or_default_layers_local_and_shared() {
        let cfg = tmp_dir("layer-cfg");
        let home = tmp_dir("layer-home");
        let _g = set_test_paths(cfg, home);

        let root = tmp_dir("layer-proj");
        // Local has only mounts.
        Config {
            mounts: vec!["~/local-mount".into()],
            ..Default::default()
        }
        .save_to_dir(&root.join(".sbx"))
        .unwrap();

        let loaded = Config::load_or_default(&root);
        assert_eq!(loaded.mounts, vec!["~/local-mount"]);
        assert!(loaded.flavor.is_none());
    }

    #[test]
    fn config_partial_round_trip_with_only_flavor() {
        let mut cfg = Config::default();
        cfg.flavor = Some("rust".into());
        let s = toml::to_string_pretty(&cfg).unwrap();
        assert!(s.contains("flavor = \"rust\""));
        assert!(!s.contains("ports"));
        assert!(!s.contains("ssh"));
        let parsed: Config = toml::from_str(&s).unwrap();
        assert_eq!(parsed.flavor.as_deref(), Some("rust"));
        assert!(parsed.ports.is_empty());
        assert!(!parsed.ssh);
    }

    #[test]
    fn extend_dedup_skips_existing_and_appends_new() {
        let mut dst = vec![1, 2, 3];
        extend_dedup(&mut dst, vec![2, 4, 3, 5]);
        assert_eq!(dst, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn extend_dedup_handles_empty_src() {
        let mut dst = vec![1, 2];
        extend_dedup::<i32>(&mut dst, vec![]);
        assert_eq!(dst, vec![1, 2]);
    }

    #[test]
    fn tunnel_right_as_string_matches_variant() {
        assert_eq!(TunnelRight::Port(8080).as_string(), "8080");
        assert_eq!(
            TunnelRight::Address("host.docker.internal:5432".into()).as_string(),
            "host.docker.internal:5432"
        );
    }

    #[test]
    fn extend_dedup_tunnels_keys_on_dir_left_right() {
        let mut dst = vec![Tunnel {
            dir: "L".into(),
            left: 8000,
            right: TunnelRight::Port(9000),
        }];
        extend_dedup_tunnels(
            &mut dst,
            vec![
                Tunnel {
                    dir: "L".into(),
                    left: 8000,
                    right: TunnelRight::Port(9000),
                },
                Tunnel {
                    dir: "L".into(),
                    left: 8000,
                    right: TunnelRight::Port(9001),
                },
                Tunnel {
                    dir: "R".into(),
                    left: 8000,
                    right: TunnelRight::Port(9000),
                },
            ],
        );
        assert_eq!(dst.len(), 3);
    }
}
