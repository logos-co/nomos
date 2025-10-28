use async_trait::async_trait;

#[async_trait]
pub trait Mode<Context> {
    async fn run(self: Box<Self>) -> Result<(ModeKind, Context), Error>;
}

pub enum ModeKind {
    Core,
    Edge,
    Broadcast,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {}
