use cyfs_base::{BuckyResult, ObjectId, ObjectMapIsolatePathOpEnvRef, ObjectMapSingleOpEnvRef};
use cyfs_core::{GroupConsensusBlock, GroupProposal};
use cyfs_lib::NONObjectInfo;

#[async_trait::async_trait]
pub trait DelegateFactory: Send + Sync {
    async fn create_rpath_delegate(
        &self,
        group_id: &ObjectId,
        rpath: &str,
        with_block: Option<&GroupConsensusBlock>,
    ) -> BuckyResult<Box<dyn RPathDelegate>>;
}

pub struct ExecuteResult {
    pub result_state_id: Option<ObjectId>, // pack block
    pub receipt: Option<NONObjectInfo>,    // to client
    pub context: Option<Vec<u8>>,          // timestamp etc.
}

#[async_trait::async_trait]
pub trait RPathDelegate: Sync + Send {
    async fn on_execute(
        &self,
        proposal: &GroupProposal,
        pre_state_id: Option<ObjectId>,
        object_map_processor: &dyn GroupObjectMapProcessor,
    ) -> BuckyResult<ExecuteResult>;

    async fn on_verify(
        &self,
        proposal: &GroupProposal,
        pre_state_id: Option<ObjectId>,
        object_map_processor: &dyn GroupObjectMapProcessor,
        execute_result: &ExecuteResult,
    ) -> BuckyResult<()>;

    async fn on_commited(
        &self,
        proposal: &GroupProposal,
        pre_state_id: Option<ObjectId>,
        object_map_processor: &dyn GroupObjectMapProcessor,
        execute_result: &ExecuteResult,
        block: &GroupConsensusBlock,
    );
}

#[async_trait::async_trait]
pub trait GroupObjectMapProcessor: Send + Sync {
    async fn create_single_op_env(&self) -> BuckyResult<ObjectMapSingleOpEnvRef>;
    async fn create_sub_tree_op_env(&self) -> BuckyResult<ObjectMapIsolatePathOpEnvRef>;
}
