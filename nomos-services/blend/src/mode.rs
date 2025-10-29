use async_trait::async_trait;

#[async_trait]
pub trait Mode<Context> {
    async fn run(self: Box<Self>) -> Result<Context, Error>;
}

#[derive(Debug, thiserror::Error)]
pub enum Error {}
