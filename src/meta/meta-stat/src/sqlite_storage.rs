use crate::storage::{Storage, map_sql_err, MetaStat, Period};
use cyfs_base::{BuckyResult, BuckyError, BuckyErrorCode, bucky_time_now, bucky_time_to_js_time, js_time_to_bucky_time};
use sqlx::sqlite::{SqlitePoolOptions, SqliteJournalMode, SqliteConnectOptions, SqliteRow};
use sqlx::{Pool, Sqlite, Transaction, Row, Executor, ConnectOptions};
use std::path::Path;
use std::time::Duration;
use log::*;
use async_trait::async_trait;
use once_cell::sync::OnceCell;

const GET_OBJ_DESC_NUM: &str = r#"SELECT count(*) from device_stat where obj_type = ?1"#;
const GET_OBJ_ADD_DESC_NUM: &str = r#"SELECT count(*) from device_stat where obj_type = ?1 and create_time >= ?2 and create_time <= ?3"#;
const GET_OBJ_ACTIVE_DESC_NUM: &str = r#"SELECT count(*) from device_stat where obj_type = ?1 and update_time >= ?2 and update_time <= ?3"#;

pub struct SqliteStorage {
    pool: OnceCell<Pool<Sqlite>>,
}

impl SqliteStorage {
    pub(crate) fn new() -> Self {
        Self {
            pool: OnceCell::new(),
        }
    }
}

#[async_trait]
impl Storage for SqliteStorage {
    async fn open(&mut self, db_path: &str) -> BuckyResult<()> {
        let database = Path::new(db_path).join("archive_db");
        info!("database: {}", database.display());
        let mut options = SqliteConnectOptions::new().filename(database.as_path())
            .journal_mode(SqliteJournalMode::Memory).busy_timeout(Duration::new(10, 0));
        options.log_statements(LevelFilter::Off);
        let pool = SqlitePoolOptions::new().max_connections(10).connect_with(options).await.map_err(map_sql_err)?;

        let _ = self.pool.set(pool);
        Ok(())
    }

    async fn init(&self) -> BuckyResult<()> {
        Ok(())
    }

    async fn get_desc(&self, obj_type: u8) -> BuckyResult<u64> {
        let row = sqlx::query(GET_OBJ_DESC_NUM).bind(obj_type).fetch_one(self.pool.get().unwrap()).await.map_err(map_sql_err)?;
        let sum: i64 = row.try_get(0).unwrap_or(0);
        Ok(sum as u64)
    }

    async fn get_desc_add(&self, obj_type: u8, period: Period) -> BuckyResult<u64> {
        let now = bucky_time_now();
        let mut start = bucky_time_to_js_time(now);
        if period == Period::Daily {
            start -= 86400 * 1000;
        } else if period == Period::Weekly {
            start -= 7 * 86400 * 1000;
        } else {
            start -= 30 * 86400 * 1000;
        }
        let start = js_time_to_bucky_time(start);

        let row = sqlx::query(GET_OBJ_ADD_DESC_NUM).bind(obj_type).bind(start as i64).bind(now as i64).fetch_one(self.pool.get().unwrap()).await.map_err(map_sql_err)?;
        let sum: i64 = row.try_get(0).unwrap_or(0);
        Ok(sum as u64)
    }

    async fn get_desc_active(&self, obj_type: u8, period: Period) -> BuckyResult<u64> {
        let now = bucky_time_now();
        let mut start = bucky_time_to_js_time(now);
        if period == Period::Daily {
            start -= 86400 * 1000;
        } else if period == Period::Weekly {
            start -= 7 * 86400 * 1000;
        } else {
            start -= 30 * 86400 * 1000;
        }
        let start = js_time_to_bucky_time(start);

        let row = sqlx::query(GET_OBJ_ACTIVE_DESC_NUM).bind(obj_type).bind(start as i64).bind(now as i64).fetch_one(self.pool.get().unwrap()).await.map_err(map_sql_err)?;
        let sum: i64 = row.try_get(0).unwrap_or(0);
        Ok(sum as u64)
    }

    async fn get_meta_api_stat(&self) -> BuckyResult<Vec<MetaStat>> {
        todo!()
    }

    async fn get_meta_object_stat(&self) -> BuckyResult<Vec<MetaStat>> {
        todo!()
    }
}