use std::{collections::HashMap, sync::Arc};

use async_std::sync::RwLock;
use cyfs_base::{
    BuckyError, BuckyErrorCode, BuckyResult, ObjectId, ObjectMap, ObjectMapOpEnvMemoryCache,
    ObjectTypeCode, RawDecode,
};
use cyfs_core::GroupConsensusBlock;
use cyfs_lib::{GlobalStateRawProcessorRef, NONObjectInfo};

use crate::{GroupRPathStatus, STATE_PATH_SEPARATOR};

#[derive(Clone)]
pub struct DecStorageCache {
    pub state: Option<ObjectId>,
    pub header_block: GroupConsensusBlock,
    pub qc_block: GroupConsensusBlock,
}

#[derive(Clone)]
pub struct DecStorage {
    cache: Arc<RwLock<Option<DecStorageCache>>>,
    pub state_processor: GlobalStateRawProcessorRef,
}

impl DecStorage {
    pub async fn load(state_processor: GlobalStateRawProcessorRef) -> BuckyResult<Self> {
        // unimplemented!();
        let obj = Self {
            cache: Arc::new(RwLock::new(None)),
            state_processor,
        };

        Ok(obj)
    }

    pub async fn cur_state(&self) -> Option<DecStorageCache> {
        let cur = self.cache.read().await;
        (*cur).clone()
    }

    pub async fn sync(
        &self,
        header_block: &GroupConsensusBlock,
        qc_block: &GroupConsensusBlock,
        remote: ObjectId,
    ) -> BuckyResult<()> {
        unimplemented!()
    }

    pub async fn get_by_path(&self, path: &str) -> BuckyResult<GroupRPathStatus> {
        unimplemented!()
    }

    pub async fn check_sub_path_value<'a>(
        &self,
        sub_path: &str,
        verifiable_status: &'a GroupRPathStatus,
    ) -> BuckyResult<Option<&'a NONObjectInfo>> {
        let block_desc = &verifiable_status.block_desc;
        let qc = &verifiable_status.certificate;

        let mut parent_state_id = match block_desc.content().result_state_id() {
            Some(state_id) => state_id.clone(),
            None => return Ok(None),
        };

        let root_cache = self.state_processor.root_cache();
        let cache = ObjectMapOpEnvMemoryCache::new_ref(root_cache.clone());

        for folder in sub_path.split(STATE_PATH_SEPARATOR) {
            let parent_state = match verifiable_status.status_map.get(&parent_state_id) {
                Some(state) => state,
                None => return Ok(None),
            };

            if ObjectTypeCode::ObjectMap != parent_state.object().obj_type_code() {
                let msg = format!(
                    "unmatch object type at path {} in folder {}, expect: ObjectMap, got: {:?}",
                    sub_path,
                    folder,
                    parent_state.object().obj_type_code()
                );
                log::warn!("{}", msg);
                return Err(BuckyError::new(BuckyErrorCode::Unmatch, msg));
            }

            let (parent, remain) = ObjectMap::raw_decode(parent_state.object_raw.as_slice())
                .map_err(|err| {
                    let msg = format!(
                        "decode failed at path {} in folder {}, {:?}",
                        sub_path, folder, err
                    );
                    log::warn!("{}", msg);
                    BuckyError::new(err.code(), msg)
                })?;

            assert_eq!(remain.len(), 0);

            let sub_map_id = parent.get_by_key(&cache, folder).await?;
            match sub_map_id {
                Some(sub_map_id) => {
                    // for next folder
                    parent_state_id = sub_map_id;
                }
                None => {
                    return Ok(None);
                }
            }
        }

        Ok(verifiable_status.status_map.get(&parent_state_id))
    }
}
