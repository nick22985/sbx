use clap::{CommandFactory, Parser};
use std::io;

use sbx::cli::{
    ClaudeCmd, Cli, Cmd, ConfigCmd, DockerCmd, EnvCmd, GuiCmd, HostProxyCmd, HostnameCmd, NetCmd,
    PortCmd, ProfileCmd, ProxyCmd, PublicCmd, ServiceCmd, SshCmd, StartCmd, TailscaleCmd,
    TunnelCmd, TunnelTopCmd, VpnCmd,
};
use sbx::commands;
use sbx::env_file;
use sbx::flavor::list_flavors;
use sbx::util::die;

fn cwd() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_else(|e| die(format!("getcwd: {e}")))
}

fn main() {
    env_file::load_into_env();
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();

    match cli.command {
        Some(Cmd::Init { private, flavor }) => commands::init::run(&cwd(), &flavor, private),
        Some(Cmd::Shell { flavor, cmd }) => {
            std::process::exit(commands::shell::from_project(&cwd(), flavor, cmd))
        }
        Some(Cmd::Run) => std::process::exit(commands::run::run(&cwd())),
        Some(Cmd::Stop) => commands::stop::run(&cwd()),
        Some(Cmd::Build { flavor }) => commands::build::run(&cwd(), false, flavor.as_deref()),
        Some(Cmd::Rebuild { flavor }) => commands::build::run(&cwd(), true, flavor.as_deref()),
        Some(Cmd::Clean { flavor }) => commands::clean::run(flavor.as_deref()),
        Some(Cmd::Purge { flavor }) => commands::purge::run(flavor.as_deref()),
        Some(Cmd::List) => {
            for f in list_flavors() {
                println!("{f}");
            }
        }
        Some(Cmd::Migrate) => commands::migrate::run(&cwd()),
        Some(Cmd::Sessions) => commands::sessions::run(),
        Some(Cmd::Config { action }) => dispatch_config(action),
        Some(Cmd::Scan { target }) => {
            let t = match target.as_str() {
                "image" => commands::scan::Target::Image,
                _ => commands::scan::Target::Fs,
            };
            commands::scan::run(&cwd(), t);
        }
        Some(Cmd::Net { action }) => match action.unwrap_or(NetCmd::Vpn { action: None }) {
            NetCmd::Vpn { action } => {
                let act = match action.unwrap_or(VpnCmd::Status) {
                    VpnCmd::Status => commands::vpn::Action::Status,
                    VpnCmd::Use { spec } => commands::vpn::Action::Use(spec),
                    VpnCmd::Auth => commands::vpn::Action::Auth,
                    VpnCmd::Inline { target } => commands::vpn::Action::Inline(target),
                    VpnCmd::Off => commands::vpn::Action::Off,
                };
                commands::vpn::run(&cwd(), act);
            }
            NetCmd::Tailscale { action } => {
                let act = match action.unwrap_or(TailscaleCmd::Status) {
                    TailscaleCmd::On { name } => commands::tailscale::Action::On(name),
                    TailscaleCmd::Off => commands::tailscale::Action::Off,
                    TailscaleCmd::Status => commands::tailscale::Action::Status,
                    TailscaleCmd::Auth { name } => commands::tailscale::Action::Auth(name),
                    TailscaleCmd::List => commands::tailscale::Action::List,
                    TailscaleCmd::Rm { name } => commands::tailscale::Action::Rm(name),
                };
                commands::tailscale::run(&cwd(), act);
            }
        },
        Some(Cmd::Proxy { action }) => {
            let act = match action.unwrap_or(ProxyCmd::Status) {
                ProxyCmd::Status => commands::proxy::Action::Status,
                ProxyCmd::Routes => commands::proxy::Action::Routes,
                ProxyCmd::Logs { follow } => commands::proxy::Action::Logs { follow },
                ProxyCmd::Stop => commands::proxy::Action::Stop,
                ProxyCmd::Mkcert => commands::proxy::Action::Mkcert,
            };
            commands::proxy::run(act);
        }
        Some(Cmd::Tunnel { action }) => {
            let act = match action.unwrap_or(TunnelTopCmd::Status) {
                TunnelTopCmd::Status => commands::tunnel::TopAction::Status,
                TunnelTopCmd::Logs { follow } => commands::tunnel::TopAction::Logs { follow },
                TunnelTopCmd::Stop => commands::tunnel::TopAction::Stop,
            };
            commands::tunnel::run_top(&cwd(), act);
        }
        Some(Cmd::HostProxy { action }) => {
            let action = action.unwrap_or(HostProxyCmd::Status);
            let act = match &action {
                HostProxyCmd::On => commands::host_proxy::Action::On,
                HostProxyCmd::Off => commands::host_proxy::Action::Off,
                HostProxyCmd::Status => commands::host_proxy::Action::Status,
                HostProxyCmd::Logs { follow } => {
                    commands::host_proxy::Action::Logs { follow: *follow }
                }
                HostProxyCmd::Stop => commands::host_proxy::Action::Stop,
                HostProxyCmd::Allow { host } => commands::host_proxy::Action::Allow(host),
                HostProxyCmd::Disallow { host } => commands::host_proxy::Action::Disallow(host),
                HostProxyCmd::List => commands::host_proxy::Action::List,
                HostProxyCmd::Reload => commands::host_proxy::Action::Reload,
            };
            commands::host_proxy::run(&cwd(), act);
        }
        Some(Cmd::Public { action }) => {
            let action = action.unwrap_or(PublicCmd::Status);
            let act = match &action {
                PublicCmd::List => commands::public::Action::List,
                PublicCmd::Add { hostname, port } => commands::public::Action::Add(hostname, port),
                PublicCmd::Rm { hostname } => commands::public::Action::Remove(hostname),
                PublicCmd::Login => commands::public::Action::Login,
                PublicCmd::Status => commands::public::Action::Status,
                PublicCmd::Logs { follow } => commands::public::Action::Logs { follow: *follow },
                PublicCmd::Stop => commands::public::Action::Stop,
            };
            commands::public::run(&cwd(), act);
        }
        Some(Cmd::Completions { shell }) => {
            clap_complete::generate(shell, &mut Cli::command(), "sbx", &mut io::stdout());
        }
        Some(Cmd::Claude {
            action,
            mounts,
            profile,
            safe,
            rc,
            docker,
            args,
        }) => match action {
            Some(ClaudeCmd::Shell) => std::process::exit(commands::claude::run(
                &cwd(),
                Vec::new(),
                true,
                mounts,
                profile,
                safe,
                rc,
                docker,
            )),

            Some(ClaudeCmd::Build) => commands::claude::build(false),
            Some(ClaudeCmd::Rebuild) => commands::claude::build(true),
            Some(ClaudeCmd::Profile { action }) => match action.unwrap_or(ProfileCmd::List) {
                ProfileCmd::List => commands::claude::print_profile_list(&cwd()),
                ProfileCmd::Add { name } => commands::claude::add_profile(&name),
                ProfileCmd::Rm { name } => commands::claude::remove_profile(&name),
                ProfileCmd::Current => {
                    commands::claude::print_current_profile(&cwd(), profile.as_deref())
                }
            },
            None => std::process::exit(commands::claude::run(
                &cwd(),
                args,
                false,
                mounts,
                profile,
                safe,
                rc,
                docker,
            )),
        },
        None => match cli.flavor {
            None => {
                let _ = Cli::command().print_help();
                println!();
            }
            Some(f) => std::process::exit(commands::shell::ad_hoc(&cwd(), &f)),
        },
    }
}

fn parse_env_set(spec: &str, value: Option<&str>) -> (String, String) {
    if let Some((k, v)) = spec.split_once('=') {
        return (k.to_string(), v.to_string());
    }
    match value {
        Some(v) => (spec.to_string(), v.to_string()),
        None => die("usage: sbx config env set KEY=VALUE  (or: sbx config env set KEY VALUE)"),
    }
}

fn dispatch_config(action: Option<ConfigCmd>) {
    let Some(action) = action else {
        let _ = Cli::command()
            .find_subcommand_mut("config")
            .map(|c| c.print_help());
        println!();
        return;
    };
    match action {
        ConfigCmd::Port { action } => {
            let action = action.unwrap_or(PortCmd::List);
            let act = match &action {
                PortCmd::List => commands::port::Action::List,
                PortCmd::Add { port } => commands::port::Action::Add(port),
                PortCmd::Rm { port } => commands::port::Action::Remove(port),
            };
            commands::port::run(&cwd(), act);
        }
        ConfigCmd::Hostname { action } => {
            let action = action.unwrap_or(HostnameCmd::List);
            let act = match &action {
                HostnameCmd::List => commands::hostname::Action::List,
                HostnameCmd::Add { hostname, port } => {
                    commands::hostname::Action::Add(hostname, port)
                }
                HostnameCmd::Rm { hostname } => commands::hostname::Action::Remove(hostname),
            };
            commands::hostname::run(&cwd(), act);
        }
        ConfigCmd::Tunnel { action } => {
            let action = action.unwrap_or(TunnelCmd::List);
            let act = match &action {
                TunnelCmd::List => commands::tunnel::Action::List,
                TunnelCmd::Add {
                    direction,
                    left,
                    right,
                } => commands::tunnel::Action::Add {
                    direction,
                    left,
                    right,
                },
                TunnelCmd::Rm { direction, left } => {
                    commands::tunnel::Action::Remove { direction, left }
                }
            };
            commands::tunnel::run(&cwd(), act);
        }
        ConfigCmd::Env { action } => match action.unwrap_or(EnvCmd::List) {
            EnvCmd::List => commands::env::run(commands::env::Action::List),
            EnvCmd::Set { spec, value } => {
                let (k, v) = parse_env_set(&spec, value.as_deref());
                commands::env::run(commands::env::Action::Set { key: &k, value: &v });
            }
            EnvCmd::Unset { key } => commands::env::run(commands::env::Action::Unset(&key)),
        },
        ConfigCmd::Start { action, rest } => match action {
            Some(StartCmd::Show) => commands::start::run(&cwd(), commands::start::Action::Show),
            Some(StartCmd::Set { cmd }) => {
                commands::start::run(&cwd(), commands::start::Action::Set(&cmd))
            }
            Some(StartCmd::Clear) => commands::start::run(&cwd(), commands::start::Action::Clear),
            None => {
                if rest.is_empty() {
                    commands::start::run(&cwd(), commands::start::Action::Show);
                } else {
                    commands::start::write_raw(&cwd(), &rest.join(" "));
                }
            }
        },
        ConfigCmd::Service { action } => {
            let action = action.unwrap_or(ServiceCmd::List);
            let act = match &action {
                ServiceCmd::List => commands::service::Action::List,
                ServiceCmd::Add { name } => commands::service::Action::Add(name),
                ServiceCmd::Rm { name } => commands::service::Action::Remove(name),
            };
            commands::service::run(&cwd(), act);
        }
        ConfigCmd::Ssh { action } => {
            let act = match action.unwrap_or(SshCmd::Status) {
                SshCmd::On => commands::ssh::Action::On,
                SshCmd::Off => commands::ssh::Action::Off,
                SshCmd::Status => commands::ssh::Action::Status,
            };
            commands::ssh::run(&cwd(), act);
        }
        ConfigCmd::Docker { action } => {
            let act = match action.unwrap_or(DockerCmd::Status) {
                DockerCmd::On => commands::docker::Action::On,
                DockerCmd::Off => commands::docker::Action::Off,
                DockerCmd::Status => commands::docker::Action::Status,
            };
            commands::docker::run(&cwd(), act);
        }
        ConfigCmd::Gui { action } => {
            let act = match action.unwrap_or(GuiCmd::Status) {
                GuiCmd::On => commands::gui::Action::On,
                GuiCmd::Off => commands::gui::Action::Off,
                GuiCmd::Status => commands::gui::Action::Status,
            };
            commands::gui::run(&cwd(), act);
        }
    }
}
