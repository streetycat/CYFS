mod client;
mod storage;
mod sqlite_storage;

use clap::{App, Arg};

use cyfs_base::BuckyResult;
use std::sync::Arc;
use crate::storage::create_storage;
use crate::client::Client;

#[macro_use]
extern crate log;

#[async_std::main]
async fn main() -> BuckyResult<()> {
    simple_logger::init_with_level(log::Level::Debug).unwrap();
    
    let matches = App::new("meta stat").version(cyfs_base::get_version())
        .arg(Arg::with_name("db_path").short("d").long("db_path").value_name("PATH").help("meta archive sqlite db path.\ndefault is current archive_db db path.").takes_value(true))
        .arg(Arg::with_name("last").short("l").long("last").value_name("LAST").help("query last month stat\ndefault is last month.").takes_value(true))
        .get_matches(); 

    let db_path = matches.value_of("db_path").unwrap_or("./");
    let deadline = matches.value_of("last").unwrap_or("1").parse::<u16>().unwrap_or(1);
    info!("dl: {}", deadline);

    // 归档按日, 周, 月 统计 sqlite直接对archive_db 数据库表操作
    let storage = Arc::new(create_storage(db_path).await.map_err(|e|{
        error!("create storage err {}", e);
        e
    })?);

    let client = Client::new(deadline, storage); 
    client.run().await;

    Ok(())
    
}