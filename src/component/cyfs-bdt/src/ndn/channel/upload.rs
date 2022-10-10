use log::*;
use std::{
    ops::Range, 
    time::Duration, 
    sync::{RwLock, atomic::{AtomicU64, Ordering}}
};
use async_std::{
    sync::Arc, 
};
use cyfs_base::*;
use crate::{
    types::*
};
use super::super::{
    chunk::*, 
    upload::*,
    types::*
};
use super::{ 
    protocol::v0::*, 
    channel::Channel, 
};

struct UploadingState {
    speed_counter: SpeedCounter,  
    history_speed: HistorySpeed, 
    pending_from: Timestamp, 
    encoder: Box<dyn ChunkEncoder>
}

struct StateImpl {
    task_state: TaskStateImpl, 
    control_state: UploadTaskControlState, 
}

enum TaskStateImpl {
    Uploading(UploadingState),
    Finished, 
    Error(BuckyErrorCode),
}


struct SessionImpl {
    chunk: ChunkId, 
    session_id: TempSeq, 
    piece_type: ChunkEncodeDesc, 
    channel: Channel, 
    state: RwLock<StateImpl>, 
    last_active: AtomicU64, 
}

#[derive(Clone)]
pub struct UploadSession(Arc<SessionImpl>);

impl std::fmt::Display for UploadSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UploadSession{{session_id:{:?}, chunk:{}, remote:{}}}", self.session_id(), self.chunk(), self.channel().remote())
    }
}

impl UploadSession {
    pub fn new(
        chunk: ChunkId, 
        session_id: TempSeq, 
        piece_type: ChunkEncodeDesc, 
        encoder: Box<dyn ChunkEncoder>, 
        channel: Channel
    ) -> Self {
        Self(Arc::new(SessionImpl {
            chunk, 
            session_id, 
            piece_type, 
            state: RwLock::new(StateImpl{
                task_state: TaskStateImpl::Uploading(UploadingState {
                    pending_from: 0, 
                    history_speed: HistorySpeed::new(0, channel.config().history_speed.clone()), 
                    speed_counter: SpeedCounter::new(0), 
                    encoder
                }), 
                control_state: UploadTaskControlState::Normal
            }), 
            channel, 
            last_active: AtomicU64::new(0), 
        }))
    }

    pub fn chunk(&self) -> &ChunkId {
        &self.0.chunk
    }

    pub fn piece_type(&self) -> &ChunkEncodeDesc {
        &self.0.piece_type
    }

    pub fn channel(&self) -> &Channel {
        &self.0.channel
    }

    pub fn session_id(&self) -> &TempSeq {
        &self.0.session_id
    }


    pub(super) fn next_piece(&self, buf: &mut [u8]) -> BuckyResult<usize> {
        let encoder = {
            let state = self.0.state.read().unwrap();
            match &state.task_state {
                TaskStateImpl::Uploading(uploading) => {
                    Some(uploading.encoder.clone_as_encoder())
                }, 
                _ => None
            }
        };
        if let Some(encoder) = encoder {
            match encoder.next_piece(self.session_id(), buf) {
                Ok(len) => {
                    let mut state = self.0.state.write().unwrap();
                    match &mut state.task_state {
                        TaskStateImpl::Uploading(uploading) => {
                            if len > 0 {
                                uploading.speed_counter.on_recv(len);
                                uploading.pending_from = 0;
                            } else {
                                uploading.pending_from = bucky_time_now();
                            }
                            Ok(len)
                        },
                        _ => {
                            Err(BuckyError::new(BuckyErrorCode::ErrorState, "not uploading"))
                        }
                    }
                   
                }, 
                Err(err) => {
                    self.cancel_by_error(BuckyError::new(err.code(), "encoder failed"));
                    Err(err)
                }
            }
        } else {
            Ok(0)
        }
    }

    pub(super) fn cancel_by_error(&self, err: BuckyError) {
        let mut state = self.0.state.write().unwrap();
        match &state.task_state {
            TaskStateImpl::Error(_) => {}, 
            _ => {
                info!("{} canceled by err:{}", self, err);
                state.task_state = TaskStateImpl::Error(err.code());
            }
        }
    }

    // 把第一个包加到重发队列里去
    pub fn on_interest(&self, _interest: &Interest) -> BuckyResult<()> {
        enum NextStep {
            ResetEncoder(Box<dyn ChunkEncoder>), 
            RespInterest(BuckyErrorCode), 
            None
        }
        self.0.last_active.store(bucky_time_now(), Ordering::SeqCst);
        let next_step = {
            let state = self.0.state.read().unwrap();
            match &state.task_state {
                TaskStateImpl::Uploading(uploading) => {
                    NextStep::ResetEncoder(uploading.encoder.clone_as_encoder())
                }, 
                TaskStateImpl::Error(err) => {
                    NextStep::RespInterest(*err)
                }, 
                _ => {
                    NextStep::None
                }
            }
        };

        match next_step {
            NextStep::ResetEncoder(encoder) => {
                debug!("{} will reset index", self);
                encoder.reset();
                Ok(())
            }, 
            NextStep::RespInterest(err) => {
                let resp_interest = RespInterest {
                    session_id: self.session_id().clone(), 
                    chunk: self.chunk().clone(), 
                    err, 
                    redirect: None,
                    redirect_referer: None,
                    to: None,
                };
                self.channel().resp_interest(resp_interest);
                Ok(())
            }, 
            NextStep::None => Ok(())
        }
    }

    pub(super) fn on_piece_control(&self, ctrl: &PieceControl) -> BuckyResult<()> {
        self.0.last_active.store(bucky_time_now(), Ordering::SeqCst);
        enum NextStep {
            MergeIndex(Box<dyn ChunkEncoder>, u32, Vec<Range<u32>>), 
            RespInterest(BuckyErrorCode), 
            None
        }

        let next_step = match ctrl.command {
            PieceControlCommand::Finish => {
                let mut state = self.0.state.write().unwrap();
                match &state.task_state {
                    TaskStateImpl::Uploading(_) => {
                        info!("{} finished", self);
                        state.task_state = TaskStateImpl::Finished;
                    }, 
                    _ => {

                    }
                }
                NextStep::None
            }, 
            PieceControlCommand::Cancel => {
                self.0.state.write().unwrap().task_state = TaskStateImpl::Error(BuckyErrorCode::Interrupted);
                info!("{} canceled by remote", self);
                NextStep::None
            }, 
            PieceControlCommand::Continue => {
                let state = self.0.state.read().unwrap();
                match &state.task_state {
                    TaskStateImpl::Uploading(uploading) => {
                        if let Some(max_index) = ctrl.max_index {
                            NextStep::MergeIndex(uploading.encoder.clone_as_encoder(), max_index, ctrl.lost_index.clone().unwrap_or_default())
                        } else {
                            NextStep::None
                        }
                    },
                    TaskStateImpl::Error(err) => NextStep::RespInterest(*err),  
                    _ => NextStep::None
                }
            },
            _ => unimplemented!()
        };

        match next_step {
            NextStep::MergeIndex(encoder, max_index, lost_index) => {
                match &ctrl.command {
                    PieceControlCommand::Continue => {
                        encoder.merge(max_index, lost_index);
                        Ok(())
                    }, 
                    _ => Ok(())
                }
            }, 
            NextStep::RespInterest(err) => {
                let resp_interest = RespInterest {
                    session_id: self.session_id().clone(), 
                    chunk: self.chunk().clone(), 
                    err: err,
                    redirect: None,
                    redirect_referer: None,
                    to: None,
                };
                self.channel().resp_interest(resp_interest);
                Ok(())
            }, 
            NextStep::None => {
                Ok(())
            }
        }
    }

    pub(super) fn on_time_escape(&self, now: Timestamp) -> Option<UploadTaskState> {
        let mut state = self.0.state.write().unwrap();
        match &mut state.task_state {
            TaskStateImpl::Uploading(uploading) => {
                if uploading.pending_from > 0 
                    && now > uploading.pending_from 
                    && Duration::from_micros(now - uploading.pending_from) > self.channel().config().resend_timeout {
                    error!("{} canceled for pending timeout", self);
                    state.task_state = TaskStateImpl::Error(BuckyErrorCode::Timeout);
                    Some(UploadTaskState::Error(BuckyErrorCode::Timeout))
                } else {
                    Some(UploadTaskState::Uploading(0))
                }
            }, 
            TaskStateImpl::Finished => None,
            TaskStateImpl::Error(err) => {
                let last_active = self.0.last_active.load(Ordering::SeqCst);
                if now > last_active 
                    && Duration::from_micros(now - last_active) > 2 * self.channel().config().msl {
                    None
                } else {
                    Some(UploadTaskState::Error(*err))
                }
            },
        }
    }
}


impl UploadTask for UploadSession {
    fn clone_as_task(&self) -> Box<dyn UploadTask> {
        Box::new(self.clone())
    }

    fn state(&self) -> UploadTaskState {
        match &self.0.state.read().unwrap().task_state {
            TaskStateImpl::Uploading(_) => UploadTaskState::Uploading(0), 
            TaskStateImpl::Finished => UploadTaskState::Finished, 
            TaskStateImpl::Error(err) => UploadTaskState::Error(*err),
        }
    }

    fn control_state(&self) -> UploadTaskControlState {
        self.0.state.read().unwrap().control_state.clone()
    }

    fn calc_speed(&self, when: Timestamp) -> u32 {
        match &mut self.0.state.write().unwrap().task_state {
            TaskStateImpl::Uploading(uploading) => {
                let cur_speed = uploading.speed_counter.update(when);
                uploading.history_speed.update(Some(cur_speed), when);
                cur_speed
            }, 
            _ => 0
        }
    }

    fn cur_speed(&self) -> u32 {
        match &self.0.state.read().unwrap().task_state {
            TaskStateImpl::Uploading(uploading) => {
                uploading.history_speed.latest()
            }, 
            _ => 0
        }
    }

    fn history_speed(&self) -> u32 {
        match &self.0.state.read().unwrap().task_state {
            TaskStateImpl::Uploading(uploading) => {
                uploading.history_speed.average()
            }, 
            _ => 0
        }
    }
}

