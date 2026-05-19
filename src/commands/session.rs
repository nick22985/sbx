use std::path::{Path, PathBuf};

use crate::docker::{self, Network, PortSpec, RunSpec};
use crate::flavor::resolve_image;
use crate::host_proxy;
use crate::mounts;
use crate::network::ProjectNetwork;
use crate::project::{port_offset, project_base_name, project_name, worktree_suffix};
use crate::proxy;
use crate::public;
use crate::service;
use crate::tailscale;
use crate::tunnel;
use crate::util::log;
use crate::vpn;

#[derive(Default)]
pub struct Cleanup {
    vpn_sidecar: Option<String>,
    tailscale_sidecar: Option<String>,
    tunnel_sidecar: Option<String>,
    tunnel_exposer: Option<String>,
    via_host_sidecar: Option<String>,
    proxy_attached: bool,
    proxy_route_project: Option<String>,
    public_project_fragment: Option<String>,
    host_proxy_attached: bool,
    host_proxy_fragment: Option<String>,
    services: Vec<String>,
    hosts_file: Option<PathBuf>,
    done: bool,
}

impl Cleanup {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn run(&mut self) {
        if self.done {
            return;
        }
        if let Some(exposer) = self.tunnel_exposer.take() {
            tunnel::stop_exposer(&exposer);
        }
        if let Some(sidecar) = self.tunnel_sidecar.take() {
            tunnel::stop_sidecar_if_idle(&sidecar);
        }
        if let Some(sidecar) = self.via_host_sidecar.take() {
            tunnel::stop_via_host_sidecar(&sidecar);
        }
        for s in self.services.drain(..) {
            service::stop_service(&s);
        }
        if let Some(sidecar) = self.tailscale_sidecar.take() {
            tailscale::stop_sidecar_if_idle(&sidecar);
        }
        if let Some(sidecar) = self.vpn_sidecar.take() {
            vpn::stop_sidecar_if_idle(&sidecar);
        }
        if let Some(name) = self.public_project_fragment.take() {
            public::delete_project_dns_routes(&name);
            public::remove_project_fragment(&name);
            public::apply_config();
        }
        if let Some(name) = self.proxy_route_project.take() {
            proxy::remove_file_route(&name);
        }
        if self.proxy_attached {
            proxy::stop_sidecar_if_idle();
            self.proxy_attached = false;
        }
        if let Some(f) = self.hosts_file.take() {
            let _ = std::fs::remove_file(f);
        }
        if let Some(name) = self.host_proxy_fragment.take() {
            host_proxy::remove_project_fragment(&name);
            let _ = host_proxy::apply_config();
        }
        if self.host_proxy_attached {
            host_proxy::stop_sidecar_if_idle();
            self.host_proxy_attached = false;
        }
        self.done = true;
    }
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        self.run();
    }
}

pub fn run_session(flavor: &str, project_root: &Path, entry: Vec<String>) -> i32 {
    docker::ensure_ssh_agent_ready(project_root);
    let image = resolve_image(flavor, project_root, false);
    let mut cleanup = Cleanup::new();

    let mut routes = proxy::read_routes(project_root);
    let public_routes = public::read_public(project_root);
    for pr in &public_routes {
        if !routes
            .iter()
            .any(|r| r.hostname == pr.hostname && r.path.is_none())
        {
            routes.push(proxy::Route {
                hostname: pr.hostname.clone(),
                path: None,
                port: pr.port,
                force_http: true,
            });
        }
    }
    let net = ProjectNetwork::read(project_root);
    let tunnels = tunnel::read_tunnels(project_root);
    let tunnel_publish = tunnel::publish_args(&tunnels);

    let mut publish = PortSpec::from_project(project_root);
    if !routes.is_empty() {
        let claimed = proxy::hostname_ports(&routes);
        publish.ports.retain(|p| !claimed.contains(p));
    }
    let mut netns_owner: Option<String> = None;

    let mut vpn_extra_subnets: Vec<String> = Vec::new();
    let mut vpn_attach_nets: Vec<String> = Vec::new();
    if !routes.is_empty() {
        proxy::ensure_network();
        if let Some(sn) = proxy::network_subnet() {
            vpn_extra_subnets.push(sn);
        }
        vpn_attach_nets.push(proxy::NETWORK.to_string());
    }
    if let Some(spec) = &net.vpn {
        let args = publish.to_docker_args();
        let sidecar = vpn::start_sidecar(
            spec,
            project_root,
            &args,
            &vpn_extra_subnets,
            &vpn_attach_nets,
        );
        cleanup.vpn_sidecar = Some(sidecar.clone());
        netns_owner = Some(sidecar);
        publish = PortSpec::default();
    }
    if net.tailscale.enabled() {
        let args = if netns_owner.is_none() {
            publish.to_docker_args()
        } else {
            Vec::new()
        };
        let ts = tailscale::start_sidecar(
            project_root,
            netns_owner.as_deref(),
            net.tailscale.profile(),
            &args,
        );
        cleanup.tailscale_sidecar = Some(ts.clone());
        if netns_owner.is_none() {
            netns_owner = Some(ts);
            publish = PortSpec::default();
        }
    }

    for svc in service::project_services(project_root) {
        let mut svc_ports: Vec<String> = Vec::new();
        if netns_owner.is_none() {
            svc_ports = publish.to_docker_args();
            svc_ports.extend(tunnel_publish.iter().cloned());
            publish = PortSpec::default();
        }
        let cname = service::start_service(&svc, project_root, netns_owner.as_deref(), &svc_ports);
        cleanup.services.push(cname.clone());
        if netns_owner.is_none() {
            netns_owner = Some(cname);
        }
    }

    let needs_socat = tunnel::needs_in_netns_forwarder(&tunnels);
    if needs_socat || (netns_owner.is_none() && !tunnels.is_empty()) {
        let mut tun_publish: Vec<String> = Vec::new();
        if netns_owner.is_none() {
            tun_publish = publish.to_docker_args();
            tun_publish.extend(tunnel_publish.iter().cloned());
            publish = PortSpec::default();
        }
        let t = tunnel::start_sidecar(project_root, &tunnels, netns_owner.as_deref(), &tun_publish);
        if !t.is_empty() {
            cleanup.tunnel_sidecar = Some(t.clone());
            if netns_owner.is_none() {
                netns_owner = Some(t);
            }
        }
    }

    let pname = project_name(project_root);
    let needs_exposer = (cleanup.vpn_sidecar.is_some() || cleanup.tailscale_sidecar.is_some())
        && !tunnel_publish.is_empty();
    if needs_exposer
        && let Some(owner) = netns_owner.as_deref()
        && let Some(exposer) = tunnel::start_exposer(&pname, owner, &tunnels)
    {
        cleanup.tunnel_exposer = Some(exposer);
    }

    if let Some(via_host) = tunnel::start_via_host_sidecar(project_root, &tunnels) {
        cleanup.via_host_sidecar = Some(via_host);
    }

    let mut extra_host_args: Vec<String> = Vec::new();
    if host_proxy::is_enabled(project_root) {
        let allowed = host_proxy::read_allowed_hosts(project_root);
        if let Err(e) = host_proxy::write_project_fragment(&pname, &allowed) {
            log(format!("host-proxy: {e}"));
            cleanup.run();
            return 1;
        }
        cleanup.host_proxy_fragment = Some(pname.clone());
        if let Err(e) = host_proxy::start_sidecar() {
            log(format!("host-proxy: {e}"));
            cleanup.run();
            return 1;
        }
        cleanup.host_proxy_attached = true;
        let _ = host_proxy::apply_config();
        for a in host_proxy::proxy_env_args() {
            extra_host_args.push(a);
        }
        if allowed.is_empty() {
            log(format!(
                "host-proxy: routing sandbox HTTPS through http://host.docker.internal:{} (unrestricted)",
                host_proxy::PORT
            ));
        } else {
            log(format!(
                "host-proxy: routing sandbox HTTPS through http://host.docker.internal:{}; allowlist: {}",
                host_proxy::PORT,
                allowed.join(", ")
            ));
        }
    }
    let network = match &netns_owner {
        Some(owner) => {
            let bridge_gw = docker::bridge_gateway();
            let hosts_path = std::env::temp_dir().join(format!(
                "sbx-hosts.{}.{}",
                std::process::id(),
                rand_suffix()
            ));
            let body = format!(
                "127.0.0.1   localhost\n\
                 ::1         localhost ip6-localhost ip6-loopback\n\
                 {bridge_gw}  host.docker.internal\n"
            );
            if std::fs::write(&hosts_path, body).is_ok() {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&hosts_path, std::fs::Permissions::from_mode(0o644));
                extra_host_args.push("-v".into());
                extra_host_args.push(format!("{}:/etc/hosts:ro", hosts_path.display()));
                cleanup.hosts_file = Some(hosts_path);
            }
            if !routes.is_empty() {
                proxy::start_sidecar();
                cleanup.proxy_attached = true;
                if let Err(e) = proxy::attach_container(owner) {
                    log(e);
                    cleanup.run();
                    return 1;
                }
                proxy::write_file_route(&pname, owner, &routes);
                cleanup.proxy_route_project = Some(pname.clone());
                for r in &routes {
                    log(format!("  http://{}/  ->  {owner}:{}", r.hostname, r.port));
                }
            }
            Network::ShareWith(owner.as_str())
        }
        None if !routes.is_empty() => {
            proxy::start_sidecar();
            cleanup.proxy_attached = true;
            for r in &routes {
                log(format!("  http://{}/  ->  :{}", r.hostname, r.port));
            }
            Network::UserDefined(proxy::NETWORK)
        }
        None => Network::Bridge,
    };

    if !public_routes.is_empty() {
        if let Err(e) = public::ensure_tunnel() {
            log(format!("public: {e}"));
            cleanup.run();
            return 1;
        }
        for pr in &public_routes {
            if let Err(e) = public::ensure_dns_route(&pr.hostname) {
                log(format!("public: {e}"));
            }
        }
        let hostnames: Vec<String> = public_routes.iter().map(|r| r.hostname.clone()).collect();
        if let Err(e) = public::write_project_fragment(&pname, &hostnames) {
            log(format!("public: {e}"));
        } else {
            cleanup.public_project_fragment = Some(pname.clone());
        }
        public::apply_config();
        for pr in &public_routes {
            log(format!("  https://{}/  (cloudflare tunnel)", pr.hostname));
        }
    }

    let labels = if netns_owner.is_some() {
        Vec::new()
    } else {
        proxy::labels_for(&pname, &routes)
    };

    let local_hosts: Vec<String> = routes
        .iter()
        .filter(|r| !public_routes.iter().any(|p| p.hostname == r.hostname))
        .map(|r| r.hostname.clone())
        .collect();
    let public_hosts: Vec<String> = public_routes.iter().map(|r| r.hostname.clone()).collect();
    let mut extra_env: Vec<(String, String)> = vec![
        ("SBX_PROJECT".into(), pname.clone()),
        ("SBX_PROJECT_BASE".into(), project_base_name(project_root)),
        (
            "SBX_WORKTREE".into(),
            worktree_suffix(project_root).unwrap_or_default(),
        ),
    ];
    let primary_port = routes
        .iter()
        .map(|r| r.port)
        .chain(public_routes.iter().map(|r| r.port))
        .next();
    if let Some(p) = primary_port {
        let offset = port_offset(project_root);
        extra_env.push(("PORT".into(), p.to_string()));
        extra_env.push(("SBX_PORT".into(), p.to_string()));
        if offset > 0 {
            log(format!(
                "worktree port offset +{offset}: app PORT={p} (declared port shifted to avoid collision in shared netns)"
            ));
        }
    }
    let primary_host = public_hosts
        .first()
        .or_else(|| local_hosts.first())
        .cloned();
    let all_hosts: Vec<String> = public_hosts
        .iter()
        .chain(local_hosts.iter())
        .cloned()
        .collect();
    if let Some(h) = &primary_host {
        extra_env.push(("SBX_HOSTNAME".into(), h.clone()));
    }
    if !all_hosts.is_empty() {
        extra_env.push(("SBX_HOSTNAMES".into(), all_hosts.join(",")));
    }
    if let Some(h) = local_hosts.first() {
        extra_env.push(("SBX_LOCAL_HOSTNAME".into(), h.clone()));
    }
    if !local_hosts.is_empty() {
        extra_env.push(("SBX_LOCAL_HOSTNAMES".into(), local_hosts.join(",")));
    }
    if let Some(h) = public_hosts.first() {
        extra_env.push(("SBX_PUBLIC_HOSTNAME".into(), h.clone()));
    }
    if !public_hosts.is_empty() {
        extra_env.push(("SBX_PUBLIC_HOSTNAMES".into(), public_hosts.join(",")));
    }

    let container_home = crate::flavor::flavor_container_home(flavor);
    let extra_mounts = mounts::resolve(project_root, &container_home, &[], Some(flavor));
    let spec = RunSpec {
        image: &image,
        flavor,
        project_root,
        entry,
        network,
        use_hostname: netns_owner.is_none(),
        publish_ports: publish,
        extra_host_args,
        extra_mounts,
        container_home,
        labels,
        mount_docker_socket: docker::project_docker_enabled(project_root),
        extra_env,
    };
    let code = docker::run_container(spec);
    cleanup.run();
    code
}

fn rand_suffix() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    format!("{nanos:08x}")
}
