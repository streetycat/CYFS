use cyfs_base::*;
use cyfs_lib::*;

use std::sync::Arc;

pub(crate) struct ObjectMapNOCCacheAdapter {
    noc: NamedObjectCacheRef,
    device_id: DeviceId,
}

impl ObjectMapNOCCacheAdapter {
    pub fn new(device_id: &DeviceId, noc: NamedObjectCacheRef) -> Self {
        Self {
            device_id: device_id.to_owned(),
            noc,
        }
    }

    pub fn new_noc_cache(
        device_id: &DeviceId,
        noc: NamedObjectCacheRef,
    ) -> ObjectMapNOCCacheRef {
        let ret = Self::new(device_id, noc);
        Arc::new(Box::new(ret) as Box<dyn ObjectMapNOCCache>)
    }
}

#[async_trait::async_trait]
impl ObjectMapNOCCache for ObjectMapNOCCacheAdapter {
    async fn exists(&self, object_id: &ObjectId) -> BuckyResult<bool> {
        // TODO noc支持exists方法
        
        let noc_req = NamedObjectCacheExistsObjectRequest {
            object_id: object_id.clone(),
            source: RequestSourceInfo::new_local_system(),
        };

        let resp = self.noc.exists_object(&noc_req).await.map_err(|e| {
            error!("exists object map from noc error! id={}, {}", object_id, e);
            e
        })?;

        if resp.meta && resp.object {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn get_object_map(&self, object_id: &ObjectId) -> BuckyResult<Option<ObjectMap>> {
        let noc_req = NamedObjectCacheGetObjectRequest {
            source: RequestSourceInfo::new_local_system(),
            object_id: object_id.clone(),
            last_access_rpath: None,
        };

        let resp = self.noc.get_object(&noc_req).await.map_err(|e| {
            error!("load object map from noc error! id={}, {}", object_id, e);
            e
        })?;

        match resp {
            Some(resp) => {
                match ObjectMap::raw_decode(&resp.object.object_raw) {
                    Ok((obj, _)) => {
                        // 首次加载后，直接设置id缓存，减少一次id计算
                        obj.direct_set_object_id_on_init(object_id);

                        Ok(Some(obj))
                    }
                    Err(e) => {
                        error!("decode ObjectMap object error: id={}, {}", object_id, e);
                        Err(e)
                    }
                }
            }
            None => Ok(None),
        }
    }

    async fn put_object_map(&self, object_id: ObjectId, object: ObjectMap) -> BuckyResult<()> {
        let dec_id = object.desc().dec_id().to_owned();

        let source = RequestSourceInfo::new_local_dec(dec_id);

        let object_raw = object.to_vec().unwrap();
        let object = AnyNamedObject::Standard(StandardObject::ObjectMap(object));
        let object = NONObjectInfo::new(object_id, object_raw, Some(Arc::new(object)));
 
        let req = NamedObjectCachePutObjectRequest {
            source,
            object,
            storage_category: NamedObjectStorageCategory::Storage,
            context: None,
            last_access_rpath: None,
            access_string: Some(AccessString::dec_default().value()),
        };

        self.noc.put_object(&req).await.map_err(|e| {
            error!(
                "insert object map to noc error! id={}, dec={:?}, {}",
                object_id, dec_id, e
            );
            e
        })?;

        Ok(())
    }
}
