use std::ffi::{CString, c_char};

use lb_c_macros::panic_to_error;
use lb_node::TracingService;
use lb_tracing_service::{TracingMessage, TracingOpenMetricsError};
use tokio::sync::oneshot;

use crate::{
    LogosBlockchainNode, OperationStatus,
    errors::OperationStatusCode,
    result::{FfiStatusResult, StatusResult},
    return_error_if_null_pointer, unwrap_or_return_error,
};

/// Reads the node's current metrics as an `OpenMetrics` text document.
///
/// This is a synchronous wrapper around the asynchronous
/// [`TracingMessage::ReadOpenMetrics`] request.
///
/// # Arguments
///
/// - `node`: A [`LogosBlockchainNode`] instance.
///
/// # Returns
///
/// A `Result` containing the rendered document on success, or an
/// [`OperationStatus`] error on failure. If `OpenMetrics` is not enabled, the
/// error code is `NotFound`.
pub(crate) fn get_open_metrics_sync(node: &LogosBlockchainNode) -> StatusResult<String> {
    let runtime_handle = node.get_runtime_handle()?;
    let overwatch_handle = node.get_overwatch_handle();

    runtime_handle.block_on(async move {
        let relay = overwatch_handle
            .relay::<TracingService>()
            .await
            .map_err(|e| {
                OperationStatus::error(
                    OperationStatusCode::RelayError,
                    format!("Failed to get relay to TracingService: {e}"),
                )
            })?;

        let (reply_channel, receiver) = oneshot::channel();
        relay
            .send(TracingMessage::ReadOpenMetrics { reply_channel })
            .await
            .map_err(|error| {
                OperationStatus::error(
                    OperationStatusCode::ChannelSendError,
                    format!("Failed to send OpenMetrics request: {error}"),
                )
            })?;

        receiver
            .await
            .map_err(|error| {
                OperationStatus::error(
                    OperationStatusCode::ChannelReceiveError,
                    format!("Failed to receive OpenMetrics response: {error}"),
                )
            })?
            .map_err(|error| match error {
                TracingOpenMetricsError::NotEnabled => {
                    OperationStatus::error(OperationStatusCode::NotFound, error.to_string())
                }
                TracingOpenMetricsError::Read(_) => {
                    OperationStatus::error(OperationStatusCode::RuntimeError, error.to_string())
                }
            })
    })
}

/// Result type for [`get_open_metrics`].
/// On success, `value` is a pointer to a NUL-terminated C string holding the
/// rendered `OpenMetrics` document.
pub type FfiGetOpenMetricsResult = FfiStatusResult<*mut c_char>;

/// Renders the node's current metrics as an `OpenMetrics` text document.
///
/// # Arguments
///
/// - `node`: A non-null pointer to a [`LogosBlockchainNode`].
///
/// # Returns
///
/// A [`FfiGetOpenMetricsResult`] containing a pointer to an allocated C string
/// on success, or an [`OperationStatus`] error on failure.
/// If `OpenMetrics` is not enabled, the error code is `NotFound`.
///
/// # Safety
///
/// This function is unsafe because it dereferences raw pointers.
/// The caller must ensure that `node` is non-null and points to a valid
/// [`LogosBlockchainNode`].
///
/// # Memory Management
///
/// This function allocates memory for the output C string.
/// The caller must free this memory using the
/// [`free_cstring`](super::free_cstring) function.
#[must_use]
#[panic_to_error]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn get_open_metrics(
    node: *const LogosBlockchainNode,
) -> FfiGetOpenMetricsResult {
    return_error_if_null_pointer!(node);
    let node = unsafe { &*node };
    let open_metrics = unwrap_or_return_error!(get_open_metrics_sync(node));

    match CString::new(open_metrics) {
        Ok(open_metrics) => FfiGetOpenMetricsResult::ok(open_metrics.into_raw()),
        Err(error) => FfiGetOpenMetricsResult::err(OperationStatus::error(
            OperationStatusCode::RuntimeError,
            format!("Failed to create CString: {error}"),
        )),
    }
}
