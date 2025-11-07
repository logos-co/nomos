use async_trait::async_trait;

use super::{DynError, RunContext};

#[async_trait]
pub trait Expectation: Send + Sync {
    fn name(&self) -> &str;

    async fn evaluate(&self, ctx: &RunContext) -> Result<(), DynError>;
}
