use std::cmp::Ordering;

use nomos_sdp_core::DeclarationId;

#[derive(Copy, Clone, PartialEq, Eq)]
pub(super) struct Participant {
    pub participation: usize,
    pub declaration_id: DeclarationId,
}

impl PartialOrd for Participant {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Participant {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.participation, self.declaration_id).cmp(&(other.participation, other.declaration_id))
    }
}
