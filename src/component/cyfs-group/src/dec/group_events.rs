use cyfs_base::{
    BuckyError, BuckyErrorCode, BuckyResult, NamedObject, ObjectDesc, ObjectId, RawConvertTo,
    RawDecode,
};
use cyfs_core::{GroupConsensusBlock, GroupProposal};
use cyfs_group_lib::{
    ExecuteResult, GroupCommand, GroupCommandCommited, GroupCommandExecute,
    GroupCommandExecuteResult, GroupCommandVerify,
};
use cyfs_lib::NONObjectInfo;

use crate::NONDriverHelper;

#[derive(Clone)]
pub(crate) struct RPathEventNotifier {
    non_driver: NONDriverHelper,
}

impl RPathEventNotifier {
    pub fn new(driver: NONDriverHelper) -> Self {
        Self { non_driver: driver }
    }

    pub async fn on_execute(
        &self,
        proposal: GroupProposal,
        prev_state_id: Option<ObjectId>,
    ) -> BuckyResult<ExecuteResult> {
        let cmd = GroupCommandExecute {
            proposal,
            prev_state_id,
        };

        let cmd = GroupCommand::from(cmd);
        let result = self
            .non_driver
            .post_object(
                NONObjectInfo {
                    object_id: cmd.desc().object_id(),
                    object_raw: cmd.to_vec()?,
                    object: None,
                },
                None,
            )
            .await?;

        assert!(result.is_some());
        match result.as_ref() {
            Some(result) => {
                let (cmd, _remain) = GroupCommand::raw_decode(result.object_raw.as_slice())?;
                assert_eq!(_remain.len(), 0);
                let mut cmd = TryInto::<GroupCommandExecuteResult>::try_into(cmd)?;
                Ok(ExecuteResult {
                    result_state_id: cmd.result_state_id.take(),
                    receipt: cmd.receipt.take(),
                    context: cmd.context.take(),
                })
            }
            None => Err(BuckyError::new(
                BuckyErrorCode::Unknown,
                "expect some result from dec-app",
            )),
        }
    }

    pub async fn on_verify(
        &self,
        proposal: GroupProposal,
        prev_state_id: Option<ObjectId>,
        execute_result: &ExecuteResult,
    ) -> BuckyResult<()> {
        let cmd = GroupCommandVerify {
            proposal,
            prev_state_id,
            result_state_id: execute_result.result_state_id.clone(),
            receipt: execute_result.receipt.clone(),
            context: execute_result.context.clone(),
        };

        let cmd = GroupCommand::from(cmd);
        let result = self
            .non_driver
            .post_object(
                NONObjectInfo {
                    object_id: cmd.desc().object_id(),
                    object_raw: cmd.to_vec()?,
                    object: None,
                },
                None,
            )
            .await?;

        assert!(result.is_none());
        Ok(())
    }

    pub async fn on_commited(
        &self,
        proposal: GroupProposal,
        prev_state_id: Option<ObjectId>,
        execute_result: &ExecuteResult,
        block: GroupConsensusBlock,
    ) {
        let cmd = GroupCommandCommited {
            proposal,
            prev_state_id,
            result_state_id: execute_result.result_state_id.clone(),
            receipt: execute_result.receipt.clone(),
            context: execute_result.context.clone(),
            block,
        };

        let cmd = GroupCommand::from(cmd);
        let result = self
            .non_driver
            .post_object(
                NONObjectInfo {
                    object_id: cmd.desc().object_id(),
                    object_raw: cmd.to_vec().expect(
                        format!("on_commited {} failed for encode", self.non_driver.dec_id())
                            .as_str(),
                    ),
                    object: None,
                },
                None,
            )
            .await
            .map_err(|err| log::warn!("on_commited {} failed {:?}", self.non_driver.dec_id(), err));

        assert!(result.is_err() || result.unwrap().is_none());
    }
}
