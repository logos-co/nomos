use std::collections::{BTreeSet, HashMap};

use nomos_core::{
    block::SessionNumber,
    sdp::{FinalizedBlockEventUpdate, Locator, ProviderId},
};

#[derive(Debug, Clone)]
pub struct Session {
    pub session_number: SessionNumber,
    pub providers: HashMap<ProviderId, BTreeSet<Locator>>,
}

impl Session {
    pub fn apply_update(&mut self, update: &FinalizedBlockEventUpdate) {
        match update.state {
            nomos_core::sdp::FinalizedDeclarationState::Active => {
                self.providers
                    .insert(update.provider_id, update.locators.clone());
            }
            nomos_core::sdp::FinalizedDeclarationState::Inactive
            | nomos_core::sdp::FinalizedDeclarationState::Withdrawn => {
                self.providers.remove(&update.provider_id);
            }
        }
    }
}
