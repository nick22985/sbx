use std::path::{Path, PathBuf};

use crate::docker::{self, Network, PortSpec, RunSpec};
use crate::flavor::resolve_image;
use crate::network::ProjectNetwork;
use crate::project::project_name;
use crate::proxy;
use crate::service;
use crate::tailscale;
use crate::util::log;
use crate::vpn;

#[derive(Default)]
pub struct Cleanup {
    vpn_sidecar: Option<String>,
    tailscale_sidecar: Option<String>,
    proxy_attached: bool,
    proxy_route_project: Option<String>,
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
        for s in self.services.drain(..) {
            service::stop_service(&s);
        }
        if let Some(sidecar) = self.tailscale_sidecar.take() {
            tailscale::stop_sidecar_if_idle(&sidecar);
        }
        if let Some(sidecar) = self.vpn_sidecar.take() {
            vpn::stop_sidecar_if_idle(&sidecar);
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

    let routes = proxy::read_routes(project_root);
    let net = ProjectNetwork::read(project_root);

    let mut publish = PortSpec::from_project(project_root);
    if !routes.is_empty() {
        let claimed = proxy::hostname_ports(&routes);
        publish.ports.retain(|p| !claimed.contains(p));
    }
    let mut netns_owner: Option<String> = None;

    if let Some(spec) = &net.vpn {
        let sidecar = vpn::start_sidecar(spec, project_root);
        cleanup.vpn_sidecar = Some(sidecar.clone());
        netns_owner = Some(sidecar);
        publish = PortSpec::default();
    }
    if net.tailscale.enabled() {
        let ts = tailscale::start_sidecar(
            project_root,
            netns_owner.as_deref(),
            net.tailscale.profile(),
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
            publish = PortSpec::default();
        }
        let cname = service::start_service(&svc, project_root, netns_owner.as_deref(), &svc_ports);
        cleanup.services.push(cname.clone());
        if netns_owner.is_none() {
            netns_owner = Some(cname);
        }
    }

    let mut extra_host_args: Vec<String> = Vec::new();
    let pname = project_name(project_root);
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
                proxy::attach_container(owner);
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

    let labels = if netns_owner.is_some() {
        Vec::new()
    } else {
        proxy::labels_for(&pname, &routes)
    };

    let spec = RunSpec {
        image: &image,
        flavor,
        project_root,
        entry,
        network,
        use_hostname: netns_owner.is_none(),
        publish_ports: publish,
        extra_host_args,
        extra_mounts: Vec::new(),
        container_home: PathBuf::from("/home/dev"),
        labels,
        mount_docker_socket: docker::project_docker_enabled(project_root),
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
