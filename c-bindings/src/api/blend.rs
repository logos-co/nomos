use std::ffi::{CString, c_char};

use lb_blend_service::message::NetworkInfo;
use lb_c_macros::panic_to_error;
use lb_core::{
    mantle::NoteId,
    sdp::{self, Locator},
};
use lb_groth16::fr_from_bytes;
use lb_node::{RuntimeServiceId, generic_services::blend::BlendService};
use lb_sdp_service::SdpSubmission;
use multiaddr::PeerId;

use crate::{
    LogosBlockchainNode,
    api::lifecycle::resolve_run_config,
    errors::{OperationStatus, OperationStatusCode},
    result::{FfiStatusResult, StatusResult},
    return_error_if_null_pointer, unwrap_or_return_error,
};

pub const KEY_SIZE: usize = 32;

#[repr(C)]
#[derive(Default)]
pub struct DeclarationId(pub [u8; KEY_SIZE]);

impl From<sdp::DeclarationId> for DeclarationId {
    fn from(id: sdp::DeclarationId) -> Self {
        Self(id.0)
    }
}

unsafe fn parse_service_note_id(ptr: *const u8) -> Result<NoteId, OperationStatus> {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, KEY_SIZE) };
    fr_from_bytes(bytes).map(NoteId).map_err(|_| {
        OperationStatus::error(
            OperationStatusCode::ValidationError,
            "Invalid `service_note_id` bytes.",
        )
    })
}

unsafe fn parse_locator(ptr: *const c_char) -> Result<Locator, OperationStatus> {
    let c_str = unsafe { std::ffi::CStr::from_ptr(ptr) };
    let Ok(s) = c_str.to_str() else {
        return Err(OperationStatus::error(
            OperationStatusCode::ValidationError,
            "`locator` is not valid UTF-8.",
        ));
    };
    s.parse::<Locator>().map_err(|_| {
        OperationStatus::error(
            OperationStatusCode::ValidationError,
            "`locator` is not a valid locator.",
        )
    })
}

/// Joins the Blend network as a core node.
///
/// This delegates to the Blend service, which derives the node's own
/// `provider_id` and `zk_id` internally, mirroring the `POST /blend/join` HTTP
/// endpoint.
///
/// # Arguments
///
/// - `node`: A non-null pointer to a running [`LogosBlockchainNode`] instance.
/// - `locator`: A non-null pointer to a locator C string.
/// - `service_note_id`: A non-null pointer to 32 bytes representing the service
///   note ID.
///
/// # Returns
///
/// A [`FfiStatusResult`] containing the declaration ID as a 32-byte array on
/// success, or an [`OperationStatus`] error on failure.
///
/// # Safety
///
/// This function is unsafe because it dereferences raw pointers.
#[panic_to_error]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn blend_join_as_core_node(
    node: *const LogosBlockchainNode,
    locator: *const c_char,
    service_note_id: *const u8,
) -> FfiStatusResult<DeclarationId> {
    return_error_if_null_pointer!(node);
    return_error_if_null_pointer!(locator);
    return_error_if_null_pointer!(service_note_id);

    let locator = unwrap_or_return_error!(unsafe { parse_locator(locator) });
    let service_note_id =
        unwrap_or_return_error!(unsafe { parse_service_note_id(service_note_id) });

    let node = unsafe { &*node };

    let runtime_handle = unwrap_or_return_error!(node.get_runtime_handle());
    let result: StatusResult<SdpSubmission> = runtime_handle.block_on(async {
        lb_api_service::http::blend::blend_join_network::<
            BlendService<RuntimeServiceId>,
            RuntimeServiceId,
        >(node.get_overwatch_handle(), locator, service_note_id)
        .await
        .map_err(|error| {
            OperationStatus::error(
                OperationStatusCode::RelayError,
                format!("Failed to join blend network: {error}"),
            )
        })
    });

    result
        .map(|submission| DeclarationId::from(submission.declaration_id))
        .into()
}

/// Gets the current Blend network information.
///
/// This is a synchronous wrapper around the asynchronous
/// [`blend_info`](lb_api_service::http::blend::blend_info) function.
///
/// # Arguments
///
/// - `node`: A [`LogosBlockchainNode`] instance.
///
/// # Returns
///
/// A `Result` containing the Blend network information on success, or an
/// [`OperationStatus`] error on failure. The value is `None` when the node has
/// not joined the Blend network as a core node.
pub(crate) fn blend_info_sync(
    node: &LogosBlockchainNode,
) -> StatusResult<Option<NetworkInfo<PeerId>>> {
    let runtime_handle = node.get_runtime_handle()?;
    let overwatch_handle = node.get_overwatch_handle();

    runtime_handle
        .block_on(lb_api_service::http::blend::blend_info::<
            BlendService<RuntimeServiceId>,
            RuntimeServiceId,
        >(overwatch_handle))
        .map_err(|error| {
            OperationStatus::error(
                OperationStatusCode::RelayError,
                format!("Failed to get blend info: {error}"),
            )
        })
}

pub type FfiBlendInfoResult = FfiStatusResult<*mut c_char>;

/// Gets the current Blend network information as a JSON string.
///
/// # Arguments
///
/// - `node`: A non-null pointer to a running [`LogosBlockchainNode`] instance.
///
/// # Returns
///
/// A [`FfiBlendInfoResult`] containing a pointer to an allocated C string (the
/// JSON-serialized network information) on success, or an [`OperationStatus`]
/// error on failure.
///
/// # Safety
///
/// This function is unsafe because it dereferences raw pointers.
///
/// # Memory Management
///
/// This function allocates memory for the output C string. The caller must
/// free this memory using the [`free_cstring`](super::free_cstring) function.
#[must_use]
#[panic_to_error]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn blend_info(node: *const LogosBlockchainNode) -> FfiBlendInfoResult {
    return_error_if_null_pointer!(node);

    let node = unsafe { &*node };
    let network_info = unwrap_or_return_error!(blend_info_sync(node));

    let json_network_info =
        unwrap_or_return_error!(serde_json::to_string(&network_info).map_err(|error| {
            OperationStatus::error(
                OperationStatusCode::RuntimeError,
                format!("Failed to serialize blend info: {error}"),
            )
        }));
    let json_network_info_c =
        unwrap_or_return_error!(CString::new(json_network_info).map_err(|error| {
            OperationStatus::error(
                OperationStatusCode::RuntimeError,
                format!("Failed to create CString: {error}"),
            )
        }));
    FfiBlendInfoResult::ok(json_network_info_c.into_raw())
}

/// Serializes a value to JSON and hands back an owned C string the caller must
/// free with [`free_cstring`](super::free_cstring).
fn to_json_cstring<T: serde::Serialize>(value: &T) -> Result<*mut c_char, OperationStatus> {
    let json = serde_json::to_string(value).map_err(|error| {
        OperationStatus::error(
            OperationStatusCode::RuntimeError,
            format!("Failed to serialize value to JSON: {error}"),
        )
    })?;
    CString::new(json).map(CString::into_raw).map_err(|error| {
        OperationStatus::error(
            OperationStatusCode::RuntimeError,
            format!("Failed to create CString: {error}"),
        )
    })
}

/// The rules a node must meet to join the Blend network as a core node.
///
/// Mirrors the referral program's `blend_requirements()`. The values are read
/// off the deployment's genesis-era parameters, so this works before a node is
/// started.
///
/// Note on omitted fields: the referral spec also lists `lock_period` and
/// `retention_period`, but neither exists as an SDP/Blend parameter today.
/// `activation_delay_epochs` is the chain-wide [`SNAPSHOT_FINALIZATION_DELAY`],
/// which is also the delay before a withdrawn note unlocks.
///
/// [`SNAPSHOT_FINALIZATION_DELAY`]: lb_core::sdp::SNAPSHOT_FINALIZATION_DELAY
#[derive(serde::Serialize)]
struct BlendRequirements {
    /// Minimum stake a single service note must hold to back a declaration.
    min_stake: u64,
    /// Epochs a core node may go without a successful activity message before
    /// its declaration is considered inactive.
    inactivity_period: u32,
    /// Epochs between a declaration landing and its stake taking effect.
    activation_delay_epochs: u32,
    /// Minimum number of core declarations the network needs for Blend rewards
    /// to be minted.
    minimum_network_size: u64,
}

pub type FfiBlendRequirementsResult = FfiStatusResult<*mut c_char>;

/// Gets the rules for joining the Blend network as a core node, as a JSON
/// string.
///
/// This reads the deployment configuration directly and does not need a running
/// node, so an onboarding flow can show the join rules before start-up. It
/// mirrors [`get_deployment_info`](super::deployment::get_deployment_info) in
/// how it resolves the configuration.
///
/// # Arguments
///
/// - `config_path`: A non-null pointer to a string holding the path to the user
///   configuration file.
/// - `custom_deployment_path`: An optional pointer to a string holding the path
///   to a custom deployment file. If null, the `DEPLOYMENT` environment
///   variable is used, falling back to the embedded default deployment.
///
/// # Returns
///
/// A [`FfiBlendRequirementsResult`] containing a pointer to an allocated C
/// string (the JSON-serialized requirements) on success, or an
/// [`OperationStatus`] error on failure.
///
/// # Safety
///
/// This function is unsafe because it dereferences raw pointers. The caller
/// must ensure that `config_path` is a valid NUL-terminated C string, and that
/// `custom_deployment_path` is either null or one as well.
///
/// # Memory Management
///
/// This function allocates memory for the output C string. The caller must
/// free this memory using the [`free_cstring`](super::free_cstring) function.
#[must_use]
#[panic_to_error]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn blend_requirements(
    config_path: *const c_char,
    custom_deployment_path: *const c_char,
) -> FfiBlendRequirementsResult {
    return_error_if_null_pointer!(config_path);

    let run_config =
        unwrap_or_return_error!(unsafe { resolve_run_config(config_path, custom_deployment_path) });

    let era_parameters = run_config.deployment.genesis_era_parameters();
    let sdp_config = &era_parameters.cryptarchia.sdp_config;

    let service_parameters = unwrap_or_return_error!(
        sdp_config
            .service_params
            .get(&sdp::ServiceType::BlendNetwork)
            .ok_or_else(|| OperationStatus::error(
                OperationStatusCode::NotFound,
                "Deployment has no Blend network service parameters.",
            ))
    );

    let requirements = BlendRequirements {
        min_stake: sdp_config.min_stake.threshold,
        inactivity_period: service_parameters.inactivity_period.into_inner().into_inner(),
        activation_delay_epochs: sdp::SNAPSHOT_FINALIZATION_DELAY.into_inner(),
        minimum_network_size: run_config
            .deployment
            .genesis_blend_reward_params()
            .minimum_network_size
            .get(),
    };

    FfiBlendRequirementsResult::ok(unwrap_or_return_error!(to_json_cstring(&requirements)))
}
