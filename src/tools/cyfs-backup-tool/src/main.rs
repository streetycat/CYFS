mod backup;
mod def;
mod restore;
mod server;
mod stack;

use cyfs_backup::BackupHttpServerMode;
use cyfs_backup_lib::*;
use cyfs_base::BuckyErrorCode;
use cyfs_util::HttpInterfaceHost;
use def::*;

use clap::{App, Arg};
use std::path::PathBuf;
use std::str::FromStr;

#[macro_use]
extern crate log;

pub const CYFS_BACKUP: &str = "cyfs-backup";

async fn main_run() {
    let matches = App::new("OOD backup & restore tools")
    .version(cyfs_base::get_version())
    .about("OOD backup & restore tools for cyfs system")
    .author("CYFS <cyfs@buckyos.com>")
    .arg(
        Arg::with_name("root")
            .long("root")
            .takes_value(true)
            .help(&format!("Specify cyfs root folder, default is {}", cyfs_util::default_cyfs_root_path().display())),
    ).arg(
        Arg::with_name("mode")
            .long("mode")
            .takes_value(true)
            .required(true)
            .help(&format!("Specify serivce mode, can be one of {}", def::ServiceMode::str_list())),
    ).arg(
        Arg::with_name("id")
            .long("id")
            .takes_value(true)
            .required_ifs(&[
                ("mode", ServiceMode::Backup.as_str()),
                ("mode", ServiceMode::Restore.as_str()),
            ])
            .help(&format!("Specify backup & restore task id, u64 in string format, Usually it can be obtained through bucky_time_now()")),
    ).arg(
        Arg::with_name("isolate")
            .long("isolate")
            .takes_value(true)
            .help("Specify isolate of cyfs root dir, default is empty string"),
    ).arg(
        Arg::with_name("target_dir")
            .long("target-dir")
            .takes_value(true)
            .help("The target directory where the backup file is stored, the default is {cyfs-root}/data/backup/{isolate}/{id}"),
    ).arg(
        Arg::with_name("file_max_size")
            .long("file-max-size")
            .takes_value(true)
            .help("The maximum size of a single backup target file in bytes, the default is 512MB"),
    ).arg(
        Arg::with_name("archive_dir")
            .long("archive-dir")
            .takes_value(true)
            .required_if("mode", ServiceMode::Restore.as_str())
            .help("The local directory where the backup file been stored"),
    ).arg(
        Arg::with_name("data-folder")
            .long("data-folder")
            .takes_value(true)
            .help("The inner data folder in archive dir, default is 'data'"),
    ).arg(
        Arg::with_name("password")
            .long("password")
            .takes_value(true)
            .help("The password used to encrypt or decrypt the target archive"),
    ).arg(
        Arg::with_name("iqf")
            .long("iqf")
            .takes_value(false)
            .help("Enable interactive query, you can query task progress and status"),
    )
    .arg(
        Arg::with_name("iqf-host")
            .long("iqf-host")
            .takes_value(true)
            .help("Specify the http address of the iqf service, which can be local/unspecified/a list of ip addresses separated by commas, and the default is local"),
    ).arg(
        Arg::with_name("exit-on-done")
            .long("exit-on-done")
            .takes_value(false)
            .help("After the Backup & restore task is completed, the process will exits. default is true, and if with --iqf option, default is false"),
    )
    .get_matches();

    // If specify the root directory, then use it
    if let Some(v) = matches.value_of("root") {
        let root = PathBuf::from_str(v).unwrap_or_else(|e| {
            error!("invalid root path: root={}, {}", v, e);
            std::process::exit(-1);
        });

        if !root.is_dir() {
            std::fs::create_dir_all(&root).unwrap_or_else(|e| {
                error!("mkdir for root path error: root={}, {}", root.display(), e);
                std::process::exit(-1);
            });
        }

        info!("root dir is {}", root.display());

        cyfs_util::bind_cyfs_root_path(root);
    }

    cyfs_util::process::check_cmd_and_exec_with_args(CYFS_BACKUP, &matches);

    cyfs_debug::CyfsLoggerBuilder::new_app(CYFS_BACKUP)
        .level("info")
        .console("info")
        .enable_bdt(Some("info"), Some("info"))
        .debug_info_flags(
            cyfs_debug::LogDebugInfoFlags::default()
                .without_args()
                .into(),
        )
        .build()
        .unwrap()
        .start();

    cyfs_debug::PanicBuilder::new("cyfs-tools", CYFS_BACKUP)
        .build()
        .start();

    let mode = matches.value_of("mode").unwrap();
    let mode = def::ServiceMode::from_str(mode)
        .map_err(|e| {
            std::process::exit(e.code().into());
        })
        .unwrap();

    let enable_iqf = matches.is_present("iqf");
    let iqf_host = if let Some(host) = matches.value_of("iqf-host") {
        HttpInterfaceHost::from_str(host)
            .map_err(|e| {
                println!("invalid iqf-host param! {}, {}", host, e);
                std::process::exit(e.code().into());
            })
            .unwrap()
    } else {
        HttpInterfaceHost::default()
    };

    let exit_on_done = if enable_iqf {
        matches.is_present("exit-on-done")
    } else {
        true
    };

    let ret = match mode {
        ServiceMode::Backup | ServiceMode::Restore => {
            let id = matches.value_of("id").unwrap();
            let isolate = matches.value_of("isolate").unwrap_or("");
            let password = match matches.value_of("password") {
                Some(pw) => Some(ProtectedPassword::new(pw)),
                None => None,
            };

            match mode {
                ServiceMode::Backup => {
                    let mut target_file = LocalFileBackupParam::default();
                    if let Some(target_dir) = matches.value_of("target_dir") {
                        target_file.dir = Some(PathBuf::from(target_dir));
                    }

                    if let Some(value) = matches.value_of("data-folder") {
                        let value = value.trim();
                        target_file.data_folder = if value.len() > 0 && value != "/" {
                            Some(value.to_owned())
                        } else {
                            None
                        };
                    }

                    if let Some(file_max_size) = matches.value_of("file_max_size") {
                        target_file.file_max_size = u64::from_str(file_max_size)
                            .map_err(|e| {
                                error!(
                                    "invalid file_max_size, must be valid u64 value: {}, {}",
                                    file_max_size, e
                                );
                                std::process::exit(BuckyErrorCode::InvalidParam.into());
                            })
                            .unwrap();
                    }

                    let params = UniBackupParams {
                        id: id.to_owned(),
                        isolate: isolate.to_owned(),
                        target_file,
                        password,
                    };

                    let backup_manager = backup::BackupService::new(&params.isolate)
                        .await
                        .map_err(|e| {
                            std::process::exit(e.code().into());
                        })
                        .unwrap();

                    if enable_iqf {
                        let interface = server::BackupInterface::new(
                            BackupHttpServerMode::GetStatusOnly,
                            Some(backup_manager.backup_manager().clone()),
                            None,
                            iqf_host,
                        );

                        if let Err(e) = interface.start().await {
                            std::process::exit(e.code().into());
                        }
                    }

                    backup_manager.backup_manager().run_uni_backup(params).await
                }
                ServiceMode::Restore => {
                    let archive = matches.value_of("archive_dir").unwrap();

                    let params = UniRestoreParams {
                        id: id.to_owned(),
                        cyfs_root: cyfs_util::get_cyfs_root_path_ref()
                            .as_os_str()
                            .to_string_lossy()
                            .to_string(),
                        isolate: isolate.to_owned(),
                        archive: PathBuf::from(archive),
                        password,
                    };

                    let restore_manager = restore::RestoreService::new(&params.isolate)
                        .await
                        .map_err(|e| {
                            std::process::exit(e.code().into());
                        })
                        .unwrap();

                    if enable_iqf {
                        let interface = server::BackupInterface::new(
                            BackupHttpServerMode::GetStatusOnly,
                            None,
                            Some(restore_manager.restore_manager().clone()),
                            iqf_host,
                        );

                        if let Err(e) = interface.start().await {
                            std::process::exit(e.code().into());
                        }
                    }

                    restore_manager
                        .restore_manager()
                        .run_uni_restore(params)
                        .await
                }
                _ => unreachable!(),
            }
        }
        ServiceMode::Interactive => Ok(()),
    };

    match ret {
        Ok(()) => {
            info!("backup service finished!!!");
            if exit_on_done {
                std::process::exit(0);
            } else {
                async_std::future::pending::<()>().await;
            }
        }
        Err(e) => {
            info!("backup service complete with error: {}", e);
            if exit_on_done {
                std::process::exit(e.code().into());
            } else {
                async_std::future::pending::<()>().await;
            }
        }
    }
}

fn main() {
    cyfs_debug::ProcessDeadHelper::patch_task_min_thread();

    async_std::task::block_on(main_run());
}
