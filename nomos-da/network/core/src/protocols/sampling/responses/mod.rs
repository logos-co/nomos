use futures::channel::oneshot::{Receiver, Sender};

use crate::protocols::sampling::{errors::SamplingError, BehaviourSampleReq, BehaviourSampleRes};

pub mod response_behaviour;

#[derive(Debug)]
pub enum SamplingEvent {
    IncomingSample {
        request_receiver: Receiver<BehaviourSampleReq>,
        response_sender: Sender<BehaviourSampleRes>,
    },
    SamplingError {
        error: SamplingError,
    },
}
