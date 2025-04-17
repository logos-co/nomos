use async_trait::async_trait;

use crate::UnitKind;

#[async_trait]
pub trait MembershipBackend {
    fn init() -> Self;

    async fn get_members_at(&self, unit_kind: UnitKind, value: u64) -> Result<Vec<String>, String>;
    async fn get_next_members(&self, unit_kind: UnitKind) -> Result<Vec<String>, String>;
}
