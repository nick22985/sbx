use std::process::Command;

use crate::util::die;

const FMT: &str = "{{.Names}}\t{{.Image}}\t{{.Status}}\t{{.Ports}}";

pub fn run() {
    let out = Command::new("docker")
        .args(["ps", "--filter", "name=^sbx-", "--format", FMT])
        .output()
        .unwrap_or_else(|e| die(format!("docker: {e}")));
    if !out.status.success() {
        die("docker ps failed");
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    let rows: Vec<Row> = stdout.lines().filter_map(parse_row).collect();
    if rows.is_empty() {
        eprintln!("sbx: no running sessions");
        return;
    }

    let widths = column_widths(&rows);
    print_header(&widths);
    for r in &rows {
        print_row(r, &widths);
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Dev,
    Service,
    Vpn,
    Tailscale,
    Tunnel,
    ViaHost,
    Proxy,
    Public,
    HostProxy,
    Other,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::Dev => "dev",
            Kind::Service => "service",
            Kind::Vpn => "vpn",
            Kind::Tailscale => "tailscale",
            Kind::Tunnel => "tunnel",
            Kind::ViaHost => "via-host",
            Kind::Proxy => "proxy",
            Kind::Public => "public",
            Kind::HostProxy => "host-proxy",
            Kind::Other => "?",
        }
    }
}

struct Row {
    kind: Kind,
    flavor: String,
    project: String,
    pid: String,
    container: String,
    image: String,
    status: String,
}

fn parse_row(line: &str) -> Option<Row> {
    let mut parts = line.splitn(4, '\t');
    let name = parts.next()?.to_string();
    let image = parts.next().unwrap_or("").to_string();
    let status = parts.next().unwrap_or("").to_string();
    let _ports = parts.next().unwrap_or("");

    let rest = name.strip_prefix("sbx-")?;
    let (kind, flavor, project, pid) = if rest == "proxy" {
        (Kind::Proxy, String::new(), String::new(), String::new())
    } else if rest == "public" {
        (Kind::Public, String::new(), String::new(), String::new())
    } else if rest == "host-proxy" {
        (Kind::HostProxy, String::new(), String::new(), String::new())
    } else if let Some(after) = rest.strip_prefix("svc-") {
        let (project, short) = after.rsplit_once('-').unwrap_or((after, ""));
        (
            Kind::Service,
            short.to_string(),
            project.to_string(),
            String::new(),
        )
    } else if let Some(after) = rest.strip_prefix("vpn-") {
        (Kind::Vpn, String::new(), after.to_string(), String::new())
    } else if let Some(after) = rest.strip_prefix("tailscale-") {
        (
            Kind::Tailscale,
            String::new(),
            after.to_string(),
            String::new(),
        )
    } else if let Some(after) = rest.strip_prefix("via-host-") {
        (
            Kind::ViaHost,
            String::new(),
            after.to_string(),
            String::new(),
        )
    } else if let Some(after) = rest.strip_prefix("tunnel-") {
        let (project, tag) = after.rsplit_once('-').unwrap_or((after, ""));
        (
            Kind::Tunnel,
            String::new(),
            project.to_string(),
            tag.to_string(),
        )
    } else {
        let (head, pid) = rest.rsplit_once('-').unwrap_or((rest, ""));
        let (flavor, project) = head.split_once('-').unwrap_or((head, ""));
        let kind = if pid.chars().all(|c| c.is_ascii_digit()) && !pid.is_empty() {
            Kind::Dev
        } else {
            Kind::Other
        };
        (
            kind,
            flavor.to_string(),
            project.to_string(),
            pid.to_string(),
        )
    };

    Some(Row {
        kind,
        flavor,
        project,
        pid,
        container: name,
        image,
        status,
    })
}

struct Widths {
    kind: usize,
    flavor: usize,
    project: usize,
    pid: usize,
    container: usize,
    image: usize,
}

fn column_widths(rows: &[Row]) -> Widths {
    let mut w = Widths {
        kind: "KIND".len(),
        flavor: "FLAVOR".len(),
        project: "PROJECT".len(),
        pid: "PID".len(),
        container: "CONTAINER".len(),
        image: "IMAGE".len(),
    };
    for r in rows {
        w.kind = w.kind.max(r.kind.as_str().len());
        w.flavor = w.flavor.max(dash_if_empty(&r.flavor).len());
        w.project = w.project.max(dash_if_empty(&r.project).len());
        w.pid = w.pid.max(dash_if_empty(&r.pid).len());
        w.container = w.container.max(r.container.len());
        w.image = w.image.max(r.image.len());
    }
    w
}

fn print_header(w: &Widths) {
    println!(
        "{:<kw$}  {:<fw$}  {:<pw$}  {:<pidw$}  {:<cw$}  {:<iw$}  STATUS",
        "KIND",
        "FLAVOR",
        "PROJECT",
        "PID",
        "CONTAINER",
        "IMAGE",
        kw = w.kind,
        fw = w.flavor,
        pw = w.project,
        pidw = w.pid,
        cw = w.container,
        iw = w.image,
    );
}

fn print_row(r: &Row, w: &Widths) {
    println!(
        "{:<kw$}  {:<fw$}  {:<pw$}  {:<pidw$}  {:<cw$}  {:<iw$}  {}",
        r.kind.as_str(),
        dash_if_empty(&r.flavor),
        dash_if_empty(&r.project),
        dash_if_empty(&r.pid),
        r.container,
        r.image,
        r.status,
        kw = w.kind,
        fw = w.flavor,
        pw = w.project,
        pidw = w.pid,
        cw = w.container,
        iw = w.image,
    );
}

fn dash_if_empty(s: &str) -> &str {
    if s.is_empty() { "-" } else { s }
}
