use std::{cmp::Ordering, collections::BTreeSet};

use crate::{versions::history_aware_refill::participant::Participant, SubnetworkId};

#[derive(Eq)]
pub(super) struct Subnetwork {
    pub participants: BTreeSet<Participant>,
    pub subnetwork_id: SubnetworkId,
}

impl PartialEq for Subnetwork {
    fn eq(&self, other: &Self) -> bool {
        self.subnetwork_id == other.subnetwork_id
    }
}

impl Ord for Subnetwork {
    fn cmp(&self, other: &Self) -> Ordering {
        self.partial_cmp(other).unwrap()
    }
}
impl PartialOrd for Subnetwork {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(
            (self.participants.len(), self.subnetwork_id)
                .cmp(&(other.participants.len(), other.subnetwork_id)),
        )
    }
}

impl Subnetwork {
    const fn new(subnetwork_id: SubnetworkId) -> Self {
        Self {
            participants: BTreeSet::new(),
            subnetwork_id,
        }
    }

    pub fn len(&self) -> usize {
        self.participants.len()
    }
}
