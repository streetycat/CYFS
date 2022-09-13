use log::*;
use std::{
    time::Duration, 
    sync::{RwLock}
};
use async_std::{
    sync::Arc, 
};
use futures::future::AbortRegistration;
use cyfs_base::*;
use crate::{
    types::*, 
    stack::{Stack, WeakStack} 
};
use super::super::{
    download::*, 
};
use super::{
    types::*, 
    protocol::v0::*, 
    provider::*,
    channel::Channel, 
};

struct InitState {
    waiters: StateWaiter, 
    history_speed: HistorySpeed
}


struct InterestingState {
    waiters: StateWaiter, 
    start_send_time: Timestamp, 
    last_send_time: Timestamp, 
    history_speed: HistorySpeed
}

struct DownloadingState {
    waiters: StateWaiter, 
    session_type: SessionType, 
    speed_counter: SpeedCounter, 
    history_speed: HistorySpeed
}

enum SessionType {
    Stream(Box<dyn DownloadSessionProvider>), 
    Raptor(Box<dyn DownloadSessionProvider>), 
}

impl SessionType {
    fn provider(&self) -> &Box<dyn DownloadSessionProvider> {
        match self {
            Self::Stream(provider) => provider,
            Self::Raptor(provider) => provider
        }
    }
}

struct FinishedState {
    send_ctrl_time: Timestamp, 
    chunk: Option<Arc<Vec<u8>>>
}

struct CanceledState {
    send_ctrl_time: Timestamp, 
    err: BuckyError
}

pub enum DownloadSessionState {
    Downloading(u32), 
    Finished,
    Canceled(BuckyErrorCode),
}

enum StateImpl {
    Init(InitState), 
    Interesting(InterestingState), 
    Downloading(DownloadingState),
    Finished(FinishedState), 
    Canceled(CanceledState),
} 

impl StateImpl {
    fn to_session_state(&self) -> DownloadSessionState {
        match self {
            Self::Init(_) => DownloadSessionState::Downloading(0), 
            Self::Interesting(_) => DownloadSessionState::Downloading(0), 
            Self::Downloading(_) => DownloadSessionState::Downloading(0), 
            Self::Finished(_) => DownloadSessionState::Finished, 
            Self::Canceled(canceled) => DownloadSessionState::Canceled(canceled.err.code()),
        }
    }
}


struct SessionImpl {
    stack: WeakStack, 
    chunk: ChunkId, 
    session_id: TempSeq, 
    channel: Channel, 
    state: RwLock<StateImpl>, 
    prefer_type: PieceSessionType, 
    referer: Option<String>,
}

#[derive(Clone)]
pub struct DownloadSession(Arc<SessionImpl>);

impl std::fmt::Display for DownloadSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DownloadSession{{session_id:{:?}, chunk:{}, remote:{}}}", self.session_id(), self.chunk(), self.channel().remote())
    }
}


impl DownloadSession {
    pub fn new(
        stack: WeakStack, 
        chunk: ChunkId, 
        session_id: TempSeq, 
        channel: Channel, 
        prefer_type: PieceSessionType,
	    referer: Option<String>, 
    ) -> Self {
        let strong_stack = Stack::from(&stack);
        Self(Arc::new(SessionImpl {
            stack, 
            chunk, 
            session_id, 
            prefer_type, 
	        referer, 
            state: RwLock::new(StateImpl::Init(InitState {
                waiters: StateWaiter::new(), 
                history_speed: HistorySpeed::new(
                    channel.initial_download_session_speed(), 
                    strong_stack.config().ndn.channel.history_speed.clone())
            })),
            channel, 
        }))
    }

    pub fn canceled(
        stack: WeakStack, 
        chunk: ChunkId, 
        session_id: TempSeq, 
        channel: Channel, 
        err: BuckyError
    ) -> Self {
        Self(Arc::new(SessionImpl {
            stack, 
            chunk, 
            session_id, 
            channel, 
            prefer_type: PieceSessionType::Unknown, 
            referer: None, 
            state: RwLock::new(StateImpl::Canceled(CanceledState {
                send_ctrl_time: 0, 
                err
            })),
        }))
    }

    pub fn chunk(&self) -> &ChunkId {
        &self.0.chunk
    }

    pub fn prefer_type(&self) -> &PieceSessionType {
        &self.0.prefer_type
    }

    pub fn referer(&self) -> Option<&String> {
        self.0.referer.as_ref()
    }

    pub fn channel(&self) -> &Channel {
        &self.0.channel
    }  

    pub fn state(&self) -> DownloadSessionState {
        (&self.0.state.read().unwrap()).to_session_state()
    }

    pub fn session_id(&self) -> &TempSeq {
        &self.0.session_id
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub fn start(&self) -> DownloadSessionState {
        self.channel().clear_dead();

        info!("{} try start", self);
        let _continue = {
            let state = &mut *self.0.state.write().unwrap();
            match state {
                StateImpl::Init(init) => {
                    let now = bucky_time_now();
                    let mut interesting = InterestingState {
                        history_speed: init.history_speed.clone(), 
                        waiters: StateWaiter::new(), 
                        start_send_time: now, 
                        last_send_time: now, 
                    };
                    std::mem::swap(&mut interesting.waiters, &mut init.waiters);
                    *state = StateImpl::Interesting(interesting);
                    true
                }, 
                _ => {
                    let err = BuckyError::new(BuckyErrorCode::ErrorState, "not in init state");
                    error!("{} try start failed for {}", self, err);
                    false
                }
            }
        };

        if _continue {
            let interest = Interest {
                session_id: self.session_id().clone(), 
                chunk: self.chunk().clone(), 
                prefer_type: self.prefer_type().clone(), 
                referer: self.referer().cloned(), 
                from: None
            };
            info!("{} sent {:?}", self, interest);
            self.channel().interest(interest);
        }

        self.state()
    }

    pub async fn wait_finish(&self) -> DownloadSessionState {
        enum NextStep {
            Wait(AbortRegistration), 
            Return(DownloadSessionState)
        }
        let next_step = {
            let state = &mut *self.0.state.write().unwrap();
            match state {
                StateImpl::Init(init) => NextStep::Wait(init.waiters.new_waiter()), 
                StateImpl::Interesting(interesting) => NextStep::Wait(interesting.waiters.new_waiter()), 
                StateImpl::Downloading(downloading) => NextStep::Wait(downloading.waiters.new_waiter()),
                StateImpl::Finished(_) => NextStep::Return(DownloadSessionState::Finished), 
                StateImpl::Canceled(canceled) => NextStep::Return(DownloadSessionState::Canceled(canceled.err.code())),
            }
        };
        match next_step {
            NextStep::Wait(waker) => StateWaiter::wait(waker, || self.state()).await,
            NextStep::Return(state) => state
        }
    }
    
    pub fn take_chunk_content(&self) -> Option<Arc<Vec<u8>>> {
        let state = &mut *self.0.state.write().unwrap();
        match state {
            StateImpl::Finished(finished) => {
                if finished.chunk.is_some() {
                    let mut chunk = None;
                    std::mem::swap(&mut chunk, &mut finished.chunk);
                    info!("{} chunk content taken", self);
                    chunk
                } else {
                    None
                }
            }, 
            _ => None
        }
    }


    pub(super) fn push_piece_data(&self, piece: &PieceData) {
        enum NextStep {
            EnterDownloading, 
            RespControl(PieceControlCommand), 
            Ignore, 
            Push(Box<dyn DownloadSessionProvider>)
        }
        use NextStep::*;
        use StateImpl::*;
        let next_step = {
            let state = &mut *self.0.state.write().unwrap();
            match state {
                Interesting(_) => {
                    EnterDownloading
                }, 
                Downloading(downloading) => {
                    downloading.speed_counter.on_recv(piece.data.len());
                    Push(downloading.session_type.provider().clone_as_provider())
                },
                Finished(finished) => {
                    let now = bucky_time_now();
                    if finished.send_ctrl_time < now 
                        && Duration::from_micros(now - finished.send_ctrl_time) > self.channel().config().resend_interval {
                        finished.send_ctrl_time = now;
                        RespControl(PieceControlCommand::Finish)
                    } else {
                        Ignore
                    }
                }, 
                Canceled(canceled) => {
                    let now = bucky_time_now();
                    if canceled.send_ctrl_time < now 
                        && Duration::from_micros(now - canceled.send_ctrl_time) > self.channel().config().resend_interval {
                        canceled.send_ctrl_time = now;
                        RespControl(PieceControlCommand::Cancel)
                    } else {
                        Ignore
                    }
                }, 
                _ => {
                    unreachable!()
                }
            }
        };

        let resp_control = |command: PieceControlCommand| {
            self.channel().send_piece_control(PieceControl {
                sequence: self.channel().gen_command_seq(), 
                session_id: self.session_id().clone(), 
                chunk: self.chunk().clone(), 
                command, 
                max_index: None, 
                lost_index: None
            })
        };

        let push_to_decoder = |provider: Box<dyn DownloadSessionProvider>| {
            if provider.push_piece_data(piece).unwrap() {
                if let Some(waiters) = {
                    let state = &mut *self.0.state.write().unwrap();
                    match state {
                        Downloading(downloading) => {
                            let mut waiters = StateWaiter::new();
                            std::mem::swap(&mut waiters, &mut downloading.waiters);
                            info!("{} finished", self);
                            *state = Finished(FinishedState {
                                send_ctrl_time: bucky_time_now(), 
                                chunk: Some(downloading.session_type.provider().decoder().chunk_content().unwrap())
                            });
                            Some(waiters)
                        }, 
                        _ => None
                    }
                } {
                    waiters.wake();
                    resp_control(PieceControlCommand::Finish)
                }
            }    
        };

        match next_step {
            EnterDownloading => {
                match *self.prefer_type() {
			//TODO: 其他session type支持
                    PieceSessionType::Stream(_) => {
                        let provider = StreamDownload::new(
                            self.chunk(), 
                            self.session_id().clone(), 
                            self.channel().clone());

                        if let Some(provider) = {
                            let state = &mut *self.0.state.write().unwrap();
                            match state {
                                Interesting(interesting) => {
                                    let mut downloading = DownloadingState {
                                        history_speed: interesting.history_speed.clone(), 
                                        speed_counter: SpeedCounter::new(piece.data.len()), 
                                        session_type: SessionType::Stream(provider.clone_as_provider()),
                                        waiters: StateWaiter::new(), 
                                    };
                                    std::mem::swap(&mut downloading.waiters, &mut interesting.waiters);
                                    *state = Downloading(downloading);
                                    Some(provider.clone_as_provider())
                                }, 
                                Downloading(downloading) => {
                                    Some(downloading.session_type.provider().clone_as_provider())
                                }, 
                                _ => None
                            }
                        } {
                            push_to_decoder(provider);
                        }
                    },
                    PieceSessionType::RaptorA(_) | PieceSessionType::RaptorB(_)  => {
                        let stack = Stack::from(&self.0.stack);
                        let view = stack.ndn().chunk_manager().view_of(self.chunk()).unwrap();
                        let decoder = view.raptor_decoder();
                        let provider = RaptorDownload::new(decoder);

                        if let Some(provider) = {
                            let state = &mut *self.0.state.write().unwrap();
                            match state {
                                Interesting(interesting) => {
                                    let mut downloading = DownloadingState {
                                        history_speed: interesting.history_speed.clone(), 
                                        speed_counter: SpeedCounter::new(piece.data.len()), 
                                        session_type: SessionType::Raptor(provider.clone_as_provider()),
                                        waiters: StateWaiter::new(), 
                                    };
                                    std::mem::swap(&mut downloading.waiters, &mut interesting.waiters);
                                    *state = Downloading(downloading);
                                    Some(provider.clone_as_provider())
                                }, 
                                Downloading(downloading) => {
                                    Some(downloading.session_type.provider().clone_as_provider())
                                }, 
                                _ => None
                            }
                        } {
                            push_to_decoder(provider);
                        }
                    },
                    _ => {
                    }
                };
            }, 
            Push(s) => {
                push_to_decoder(s)
            }, 
            RespControl(cmd) => resp_control(cmd), 
            Ignore => {}
        }
    }

    pub(super) fn on_resp_interest(&self, resp_interest: &RespInterest) -> BuckyResult<()> {
        match &resp_interest.err {
            BuckyErrorCode::Ok => unimplemented!(),
            _ => {
                self.cancel_by_error(BuckyError::new(resp_interest.err, "remote resp interest error"));
            }
        }
        Ok(())
    }

    fn resend_interest(&self) -> BuckyResult<()> {
        {
            let state = &mut *self.0.state.write().unwrap();
            match state {
                StateImpl::Interesting(interesting) => {
                    interesting.last_send_time = bucky_time_now(); 
                    Ok(())
                }, 
                _ => Err(BuckyError::new(BuckyErrorCode::ErrorState, "not in interesting state"))
            }
        }?;
        let interest = Interest {
            session_id: self.session_id().clone(), 
            chunk: self.chunk().clone(), 
            prefer_type: self.prefer_type().clone(), 
            from: None,
            referer: self.referer().cloned()
        };
        info!("{} sent {:?}", self, interest);
        self.channel().interest(interest);
        Ok(())
    }


    pub fn cancel_by_error(&self, err: BuckyError) {
        error!("{} cancel by err {}", self, err);

        let mut waiters = StateWaiter::new();
        {
            let state = &mut *self.0.state.write().unwrap();
            match state {
                StateImpl::Init(init) => {
                    std::mem::swap(&mut waiters, &mut init.waiters);
                    *state = StateImpl::Canceled(CanceledState {
                        send_ctrl_time: 0, 
                        err
                    });
                },
                StateImpl::Interesting(interesting) => {
                    std::mem::swap(&mut waiters, &mut interesting.waiters);
                    *state = StateImpl::Canceled(CanceledState {
                        send_ctrl_time: 0, 
                        err
                    });
                },
                StateImpl::Downloading(downloading) => {
                    std::mem::swap(&mut waiters, &mut downloading.waiters);
                    *state = StateImpl::Canceled(CanceledState {
                        send_ctrl_time: 0, 
                        err
                    });
                },
	    	    StateImpl::Finished(_) => {
                    *state = StateImpl::Canceled(CanceledState {
                        send_ctrl_time: 0, 
                        err
                    });
                },
                _ => {}
            };
        }
        waiters.wake();
    }

    pub(super) fn on_time_escape(&self, now: Timestamp) -> BuckyResult<()> {
        enum NextStep {
            None, 
            SendInterest, 
            Cancel, 
            CallProvider(Box<dyn DownloadSessionProvider>),
        }
        let next_step = {
            let state = &*self.0.state.read().unwrap();
            match state {
                StateImpl::Init(_) => NextStep::None, 
                StateImpl::Interesting(interesting) => {
                    if now > interesting.start_send_time
                        && Duration::from_micros(now - interesting.start_send_time) > self.channel().config().resend_timeout {
                        NextStep::Cancel
                    } else if now > interesting.last_send_time 
                        && Duration::from_micros(now - interesting.last_send_time) > self.channel().config().resend_interval {
                        NextStep::SendInterest
                    } else {
                        NextStep::None
                    }
                }, 
                StateImpl::Downloading(downloading) => {
                    NextStep::CallProvider(downloading.session_type.provider().clone_as_provider())
                },
                StateImpl::Finished(_) => NextStep::None, 
                StateImpl::Canceled(_) => NextStep::None,
            }
        };
        
        match next_step {
            NextStep::None => Ok(()), 
            NextStep::Cancel => {
                self.cancel_by_error(BuckyError::new(BuckyErrorCode::Timeout, "interest timeout"));
                Err(BuckyError::new(BuckyErrorCode::Timeout, "interest timeout"))
            }, 
            NextStep::SendInterest => {
                let _ = self.resend_interest();
                Ok(())
            }, 
            NextStep::CallProvider(provider) => {
                match self.0.prefer_type {
                    PieceSessionType::RaptorA(_) => {
                    },
                    _ => {}
                }

                match provider.on_time_escape(now) {
                    Ok(_) => {
                        Ok(())
                    },
                    Err(err) => {
                        self.cancel_by_error(err);
                        Err(BuckyError::new(BuckyErrorCode::Timeout, "session timeout"))
                    }
                }
            }
        }
    }

    pub fn calc_speed(&self, when: Timestamp) -> u32 {
        let state = &mut *self.0.state.write().unwrap();
        match state {
            StateImpl::Init(init) => {
                init.history_speed.update(Some(0), when);
                0
            },
            StateImpl::Interesting(interesting) => {
                interesting.history_speed.update(Some(0), when);
                0
            },
            StateImpl::Downloading(downloading) => {
                let cur_speed = downloading.speed_counter.update(when);
                downloading.history_speed.update(Some(cur_speed), when);
                cur_speed
            },
            _ => 0
        }
    }

    pub fn cur_speed(&self) -> u32 {
        let state = &*self.0.state.read().unwrap();
        match state {
            StateImpl::Downloading(downloading) => downloading.history_speed.latest(),
            _ => 0
        }
    }

    pub fn history_speed(&self) -> u32 {
        let state = &*self.0.state.read().unwrap();
        match state {
            StateImpl::Init(init) => init.history_speed.average(),
            StateImpl::Interesting(interesting) => interesting.history_speed.average(),
            StateImpl::Downloading(downloading) => downloading.history_speed.average(),
            _ => 0
        }
    }
}




