use super::blob::*;
use cyfs_base::*;
use cyfs_lib::*;

use std::path::{PathBuf, Path};

pub struct FileBlobStorage {
    root: PathBuf,
}

impl FileBlobStorage {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
        }
    }

    async fn get_full_path(&self, object_id: &ObjectId, auto_create: bool) -> BuckyResult<PathBuf> {
        let hash_str;
        let len;
        #[cfg(target_os = "windows")]
        {
            hash_str = hex::encode(object_id.as_slice());
            len = 3;
        }
        #[cfg(not(target_os = "windows"))]
        {
            hash_str = object_id.to_string();
            len = 2;
        }
        
        let (tmp, first) = hash_str.split_at(hash_str.len() - len);
        let (_, second) = tmp.split_at(tmp.len() - len);

        let path = self.root.join(format!("{}/{}", first, second));
        if auto_create && !path.exists() {
            async_std::fs::create_dir_all(&path).await.map_err(|e| {
                let msg = format!("create dir for object blob error! path={}, {}", path.display(), e);
                error!("{}", msg);
                BuckyError::new(BuckyErrorCode::IoError, msg)
            })?;
        }

        let path = path.join(hash_str);

        Ok(path)
    }

    async fn load_object(&self, path: &Path) -> BuckyResult<NONObjectInfo> {
        let object_raw = async_std::fs::read(&path).await.map_err(|e| {
            let msg = format!("read object blob from file error! path={}, {}", path.display(), e);
            error!("{}", msg);
            BuckyError::new(BuckyErrorCode::IoError, msg)
        })?;

        let info = NONObjectInfo::new_from_object_raw(object_raw)?;
        Ok(info)
    }
}

#[async_trait::async_trait]
impl BlobStorage for FileBlobStorage {
    async fn put_object(&self, data: NONObjectInfo) -> BuckyResult<()> {
        let path = self.get_full_path(&data.object_id, true).await?;

        async_std::fs::write(&path, &data.object_raw).await.map_err(|e| {
            let msg = format!("save object blob to file error! path={}, {}", path.display(), e);
            error!("{}", msg);
            BuckyError::new(BuckyErrorCode::IoError, msg)
        })?;

        info!("save object blob to file success! object={}", data.object_id);
        Ok(())
    }

    async fn get_object(&self, object_id: &ObjectId) -> BuckyResult<Option<NONObjectInfo>> {
        let path = self.get_full_path(object_id, false).await?;
        if !path.exists() {
            return Ok(None);
        }

        let info = self.load_object(&path).await?;

        Ok(Some(info))
    }

    async fn delete_object(&self, object_id: &ObjectId) -> BuckyResult<Option<NONObjectInfo>> {
        let path = self.get_full_path(object_id, false).await?;
        if !path.exists() {
            return Ok(None);
        }

        let info = match self.load_object(&path).await {
            Ok(info) => {
                Some(info)
            }
            Err(_) => {
                None
            }
        };

        async_std::fs::remove_file(&path).await.map_err(|e| {
            let msg = format!("remove object blob file error! path={}, {}", path.display(), e);
            error!("{}", msg);
            BuckyError::new(BuckyErrorCode::IoError, msg)
        })?;

        info!("remove object blob file success! object={}", object_id);
        Ok(info)
    }

    async fn exists_object(&self, object_id: &ObjectId) -> BuckyResult<bool> {
        let path = self.get_full_path(object_id, false).await?;
        Ok(path.exists())
    }

    async fn stat(&self) -> BuckyResult<BlobStorageStat> {
        // TODO
        let resp = BlobStorageStat {
            count: 0,
            storage_size: 0,
        };

        Ok(resp)
    }
}