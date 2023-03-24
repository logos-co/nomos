// std
// crates
use rand::Rng;
use serde::Deserialize;
// internal
use crate::node::{Node, NodeId};

#[derive(Default)]
pub struct CarnotState {}

#[derive(Clone, Deserialize)]
pub struct CarnotSettings {}

#[allow(dead_code)] // TODO: remove when handling settings
pub struct CarnotNode {
    id: NodeId,
    state: CarnotState,
    settings: CarnotSettings,
}

impl Node for CarnotNode {
    type Settings = CarnotSettings;
    type State = CarnotState;

    fn new<R: Rng>(_rng: &mut R, id: NodeId, settings: Self::Settings) -> Self {
        Self {
            id,
            state: Default::default(),
            settings,
        }
    }

    fn id(&self) -> NodeId {
        self.id
    }

    fn current_view(&self) -> usize {
        todo!()
    }

    fn state(&self) -> &CarnotState {
        &self.state
    }

    fn step(&mut self) {
        todo!()
    }
}
