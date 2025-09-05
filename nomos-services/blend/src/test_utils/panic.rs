use std::{fmt::Debug, panic};

use tokio::task::JoinHandle;

/// Expect the panic from the given async task,
/// and resume the panic, so the async test can check the panic message.
pub async fn resume_panic_from<Error>(join_handle: JoinHandle<Result<(), Error>>)
where
    Error: Debug,
{
    panic::resume_unwind(join_handle.await.unwrap_err().into_panic());
}
