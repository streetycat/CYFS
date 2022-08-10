#![windows_subsystem = "windows"]

mod config;
mod config_repo;
mod daemon;
mod monitor;
mod package;
mod repo;
mod service;

use clap::{App, Arg};
use std::str::FromStr;

use daemon::{Daemon, start_control};
use service::{ServiceMode, SERVICE_MANAGER};

#[macro_use]
extern crate log;

const SERVICE_NAME: &str = ::cyfs_base::OOD_DAEMON_NAME;


#[async_std::main]
async fn main() {
    let app = App::new("ood-daemon service")
        .version(cyfs_base::get_version())
        .about("ood-daemon service for cyfs system")
        .author("CYFS <dev@cyfs.com>")
        .arg(
            Arg::with_name("as_monitor")
                .long("as-monitor")
                .takes_value(false)
                .help("Run ood-daemon service as backend monitor service"),
        )
        .arg(
            Arg::with_name("no_monitor")
                .long("no-monitor")
                .takes_value(false)
                .help("Run ood-daemon service without backend monitor service"),
        )
        .arg(
            Arg::with_name("no_ood_control")
                .long("no-ood-control")
                .takes_value(false)
                .help("Run ood-daemon service without ood control service"),
        )
        .arg(
            Arg::with_name("mode")
                .long("mode")
                .takes_value(true)
                .default_value("daemon")
                .help("Daemon service mode, daemon|installer|vood, default is daemon"),
        )
        .arg(
            Arg::with_name("port")
                .long("port")
                .takes_value(true)
                .help("Specify OOD bind service port"),
        )
        .arg(
            Arg::with_name("host")
                .long("host")
                .takes_value(true)
                .help("Specify OOD service public address for external services and tools, installer will bind 0 addr by default"),
        )
        .arg(
            Arg::with_name("strict-host")
                .long("strict-host")
                .takes_value(true)
                .help("Specify OOD bind service public address"),
        )
        .arg(
            Arg::with_name("ipv4_only")
                .long("ipv4-only")
                .takes_value(false)
                .help("Specify OOD bind service just use ipv4 address"),
        )
        .arg(
            Arg::with_name("ipv6_only")
                .long("ipv6-only")
                .takes_value(false)
                .help("Specify OOD bind service just use ipv6 address"),
        );

    let app = cyfs_util::process::prepare_args(app);
    let matches = app.get_matches();

    let as_monitor = matches.is_present("as_monitor");
    if as_monitor {
        monitor::ServiceMonitor::run_as_monitor(SERVICE_NAME);
        return;
    }

    let no_monitor = matches.is_present("no_monitor");

    // 如果是stop命令，那么也需要尝试停止monitor进程
    if !no_monitor && matches.is_present("stop") {
        monitor::ServiceMonitor::stop_monitor_process(SERVICE_NAME);
    }

    cyfs_util::process::check_cmd_and_exec_with_args(SERVICE_NAME, &matches);

    cyfs_debug::CyfsLoggerBuilder::new_service(SERVICE_NAME)
        .level("debug")
        .console("info")
        .enable_bdt(Some("debug"), Some("debug"))
        .build()
        .unwrap()
        .start();

    cyfs_debug::PanicBuilder::new("cyfs-service", SERVICE_NAME)
        .build()
        .start();

    // ::cyfs_base::init_log_with_isolate_bdt(SERVICE_NAME, Some("trace"), Some("trace"));

    // 启动monitor服务
    if !no_monitor {
        monitor::ServiceMonitor::start_monitor(SERVICE_NAME);
    }

    // 切换到目标的服务模式
    let mode = matches.value_of("mode").unwrap();
    let mode = match ServiceMode::from_str(mode) {
        Ok(v) => v,
        Err(_) => {
            return;
        }
    };
    SERVICE_MANAGER.change_mode(mode.clone());

    let no_ood_control = matches.is_present("no_ood_control");

    if !no_ood_control {
        if let Err(e) = start_control(mode.clone(), &matches).await {
            println!("start ood control failed! {}", e);
            std::process::exit(-1);
        }
    } else {
        info!("will run without ood control service");
    }

    let daemon = Daemon::new(mode, no_monitor);
    if let Err(e) = daemon.run().await {
        error!("daemon run error! err={}", e);
    }
}
