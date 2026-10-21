use std::ffi::{CString, c_char};

use lb_blend_service::message::NetworkInfo;
use lb_c_macros::panic_to_error;
use lb_core::{
    mantle::NoteId,
    sdp::{self, Locator},
};
use lb_groth16::fr_from_bytes;
use lb_key_management_system_keys::keys::ZkPublicKey;
use lb_node::{
    RuntimeServiceId,
    generic_services::{
        CryptarchiaService, SdpMempoolAdapter, SdpRecoveryBackend, SdpWalletAdapter,
        blend::BlendService,
    },
};
use lb_sdp_service::SdpSubmission;
use multiaddr::PeerId;

use crate::{
    LogosBlockchainNode,
    api::{cryptarchia::ensure_online, time::get_time_info_sync},
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
/// off the deployment parameters of the era in force at the epoch the time
/// service currently reports, so they stay correct once more than one era is
/// scheduled.
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
/// Reads the current epoch from the time service and matches it against the
/// deployment's era schedule, so the requirements are those of the era in
/// force now rather than always the genesis era.
///
/// # Arguments
///
/// - `node`: A non-null pointer to a running [`LogosBlockchainNode`] instance.
///
/// # Returns
///
/// A [`FfiBlendRequirementsResult`] containing a pointer to an allocated C
/// string (the JSON-serialized requirements) on success, or an
/// [`OperationStatus`] error on failure.
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
pub unsafe extern "C" fn blend_requirements(
    node: *const LogosBlockchainNode,
) -> FfiBlendRequirementsResult {
    return_error_if_null_pointer!(node);

    let node = unsafe { &*node };

    // The current epoch comes from the time service (the authoritative source),
    // then selects the era in force from the deployment's era schedule, so the
    // requirements reflect the era a node joining now would fall under.
    let current_epoch = unwrap_or_return_error!(get_time_info_sync(node)).current_epoch;
    let era_parameters = node
        .deployment()
        .era_parameters_at_epoch(lb_chain_service::Epoch::new(current_epoch));
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
        minimum_network_size: era_parameters
            .blend_reward_params()
            .minimum_network_size
            .get(),
    };

    FfiBlendRequirementsResult::ok(unwrap_or_return_error!(to_json_cstring(&requirements)))
}

/// The outcome of leaving core participation.
///
/// Mirrors the referral program's `blend_withdraw()`.
#[derive(serde::Serialize)]
struct BlendWithdrawal {
    /// The hash of the submitted withdrawal transaction, if one was produced.
    tx_hash: Option<lb_core::mantle::transactions::TxHash>,
    /// The epoch at which the locked stake is expected to unlock. Estimated
    /// from the node's current epoch plus [`SNAPSHOT_FINALIZATION_DELAY`]; the
    /// ledger fixes the exact value when the withdrawal lands in a block.
    ///
    /// [`SNAPSHOT_FINALIZATION_DELAY`]: lb_core::sdp::SNAPSHOT_FINALIZATION_DELAY
    unlocks_at_epoch: u32,
}

pub type FfiBlendWithdrawResult = FfiStatusResult<*mut c_char>;

/// Withdraws this node from Blend core participation, as a JSON string.
///
/// Looks up the node's own current declaration, submits a withdrawal for it,
/// and reports the transaction hash together with the epoch the stake is
/// expected to unlock at.
///
/// # Arguments
///
/// - `node`: A non-null pointer to a running [`LogosBlockchainNode`] instance.
///
/// # Returns
///
/// A [`FfiBlendWithdrawResult`] containing a pointer to an allocated C string
/// (the JSON-serialized withdrawal outcome) on success, or an
/// [`OperationStatus`] error on failure. A `NotFound` error is returned when
/// the node has no current declaration to withdraw.
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
pub unsafe extern "C" fn blend_withdraw(node: *const LogosBlockchainNode) -> FfiBlendWithdrawResult {
    return_error_if_null_pointer!(node);

    let node = unsafe { &*node };
    let runtime_handle = unwrap_or_return_error!(node.get_runtime_handle());
    let overwatch_handle = node.get_overwatch_handle().clone();

    let submission: StatusResult<SdpSubmission> = runtime_handle.block_on(async {
        let declaration_id = lb_api_service::http::sdp::get_current_declaration_id_handler::<
            SdpMempoolAdapter<RuntimeServiceId>,
            SdpWalletAdapter<RuntimeServiceId>,
            CryptarchiaService<RuntimeServiceId>,
            SdpRecoveryBackend<RuntimeServiceId>,
            RuntimeServiceId,
        >(overwatch_handle.clone())
        .await
        .map_err(|error| {
            OperationStatus::error(
                OperationStatusCode::RelayError,
                format!("Failed to get current declaration id: {error}"),
            )
        })?
        .ok_or_else(|| {
            OperationStatus::error(
                OperationStatusCode::NotFound,
                "Node has no current Blend declaration to withdraw.",
            )
        })?;

        lb_api_service::http::sdp::post_withdrawal_handler::<
            SdpMempoolAdapter<RuntimeServiceId>,
            SdpWalletAdapter<RuntimeServiceId>,
            CryptarchiaService<RuntimeServiceId>,
            SdpRecoveryBackend<RuntimeServiceId>,
            RuntimeServiceId,
        >(overwatch_handle.clone(), declaration_id)
        .await
        .map_err(|error| {
            OperationStatus::error(
                OperationStatusCode::RelayError,
                format!("Failed to post Blend withdrawal: {error}"),
            )
        })
    });
    let submission = unwrap_or_return_error!(submission);

    // Fetched outside the block above so this sync wrapper's own `block_on` is
    // not nested inside another.
    let time_info = unwrap_or_return_error!(get_time_info_sync(node));
    let unlocks_at_epoch = time_info
        .current_epoch
        .saturating_add(sdp::SNAPSHOT_FINALIZATION_DELAY.into_inner());

    let withdrawal = BlendWithdrawal {
        tx_hash: submission.tx_id,
        unlocks_at_epoch,
    };
    FfiBlendWithdrawResult::ok(unwrap_or_return_error!(to_json_cstring(&withdrawal)))
}

pub type FfiBlendRewardsResult = FfiStatusResult<*mut c_char>;

/// Gets this epoch's Blend reward estimate for this node, as a JSON string.
///
/// Mirrors the referral program's `blend_rewards()`. The JSON holds the
/// node's reward key (its zk id) alongside the paying epoch, pool amount,
/// proof status, premium flag, estimated reward, and network sizes.
///
/// # Arguments
///
/// - `node`: A non-null pointer to a running [`LogosBlockchainNode`] instance.
///
/// # Returns
///
/// A [`FfiBlendRewardsResult`] containing a pointer to an allocated C string
/// (the JSON-serialized rewards) on success, or an [`OperationStatus`] error on
/// failure. The call fails immediately with a `ServiceError` when the node is
/// not `Online`, rather than blocking, and with `NotFound` when the Blend
/// service is not available.
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
pub unsafe extern "C" fn blend_rewards(node: *const LogosBlockchainNode) -> FfiBlendRewardsResult {
    return_error_if_null_pointer!(node);

    let node = unsafe { &*node };

    // The referral spec requires an immediate error when the node is not
    // Online, never blocking. Checked first, before any relay call.
    unwrap_or_return_error!(ensure_online(node));

    let runtime_handle = unwrap_or_return_error!(node.get_runtime_handle());
    let overwatch_handle = node.get_overwatch_handle().clone();

    let rewards_json: StatusResult<serde_json::Value> = runtime_handle.block_on(async {
        let reward_key = lb_api_service::http::blend::blend_zk_id::<
            BlendService<RuntimeServiceId>,
            RuntimeServiceId,
        >(&overwatch_handle)
        .await
        .map_err(|error| {
            OperationStatus::error(
                OperationStatusCode::RelayError,
                format!("Failed to get blend reward key: {error}"),
            )
        })?;

        let info = lb_api_service::http::blend::blend_rewards::<RuntimeServiceId>(
            &overwatch_handle,
            reward_key,
        )
        .await
        .map_err(|error| {
            OperationStatus::error(
                OperationStatusCode::RelayError,
                format!("Failed to get blend rewards: {error}"),
            )
        })?
        .ok_or_else(|| {
            OperationStatus::error(
                OperationStatusCode::NotFound,
                "Blend rewards are not available on this node.",
            )
        })?;

        // The reward key is carried alongside the ledger-provided fields, so
        // the JSON matches the referral spec's flat `blend_rewards()` shape.
        let mut value = serde_json::to_value(&info).map_err(|error| {
            OperationStatus::error(
                OperationStatusCode::RuntimeError,
                format!("Failed to serialize blend rewards: {error}"),
            )
        })?;
        if let serde_json::Value::Object(map) = &mut value {
            let reward_key_value = serde_json::to_value(reward_key).map_err(|error| {
                OperationStatus::error(
                    OperationStatusCode::RuntimeError,
                    format!("Failed to serialize blend reward key: {error}"),
                )
            })?;
            map.insert("reward_key".to_owned(), reward_key_value);
        }
        Ok(value)
    });

    let rewards_json = unwrap_or_return_error!(rewards_json);
    FfiBlendRewardsResult::ok(unwrap_or_return_error!(to_json_cstring(&rewards_json)))
}

/// Count of negotiated session peers for the current epoch, with how many are
/// healthy.
#[derive(serde::Serialize)]
struct SessionPeers {
    total: usize,
    healthy: usize,
}

/// This node's own Blend core declaration, as reported in [`BlendStatus`].
#[derive(serde::Serialize)]
struct DeclarationInfo {
    id: sdp::DeclarationId,
    provider_id: sdp::ProviderId,
    zk_id: ZkPublicKey,
    locked_note_id: NoteId,
    locators: sdp::Locators,
    created_epoch: u32,
    /// The epoch the declaration's stake takes effect, derived as
    /// `created_epoch + SNAPSHOT_FINALIZATION_DELAY`.
    active_from_epoch: u32,
    withdraw_at: Option<u32>,
}

/// A snapshot of this node's Blend participation.
///
/// Mirrors the referral program's `blend_status()`, minus the fields that
/// require logic not yet present in the node: the synthesized `state` and the
/// whole `activity` block (`this_epoch`, `reason`, `tokens_collected`,
/// `last_active_epoch`, `last_fee`).
#[derive(serde::Serialize)]
struct BlendStatus {
    /// Whether this node participates as a core node (versus edge).
    core_mode: bool,
    /// This node's own declaration, or `None` when it has not declared.
    declaration: Option<DeclarationInfo>,
    current_epoch: u32,
    session_peers: SessionPeers,
}

pub type FfiBlendStatusResult = FfiStatusResult<*mut c_char>;

/// Gets this node's Blend participation status, as a JSON string.
///
/// Reports whether the node is a core node, its own declaration (if any), the
/// current epoch, and this epoch's session peer counts. The synthesized
/// `state` and the `activity` block from the referral spec are intentionally
/// omitted, as they need tracking the node does not keep yet.
///
/// # Arguments
///
/// - `node`: A non-null pointer to a running [`LogosBlockchainNode`] instance.
///
/// # Returns
///
/// A [`FfiBlendStatusResult`] containing a pointer to an allocated C string
/// (the JSON-serialized status) on success, or an [`OperationStatus`] error on
/// failure.
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
pub unsafe extern "C" fn blend_status(node: *const LogosBlockchainNode) -> FfiBlendStatusResult {
    return_error_if_null_pointer!(node);

    let node = unsafe { &*node };

    // Core mode and session peers come from the Blend network info.
    let network_info = unwrap_or_return_error!(blend_info_sync(node));
    let (core_mode, session_peers) = network_info
        .as_ref()
        .and_then(|info| info.core_info.as_ref())
        .map_or(
            (false, SessionPeers { total: 0, healthy: 0 }),
            |core_info| {
                let total = core_info.current_epoch_peers.len();
                let healthy = core_info
                    .current_epoch_peers
                    .iter()
                    .filter(|(_, healthy)| *healthy)
                    .count();
                (true, SessionPeers { total, healthy })
            },
        );

    let current_epoch = unwrap_or_return_error!(get_time_info_sync(node)).current_epoch;

    let runtime_handle = unwrap_or_return_error!(node.get_runtime_handle());
    let overwatch_handle = node.get_overwatch_handle().clone();

    let declaration: StatusResult<Option<DeclarationInfo>> = runtime_handle.block_on(async {
        let Some(declaration_id) =
            lb_api_service::http::sdp::get_current_declaration_id_handler::<
                SdpMempoolAdapter<RuntimeServiceId>,
                SdpWalletAdapter<RuntimeServiceId>,
                CryptarchiaService<RuntimeServiceId>,
                SdpRecoveryBackend<RuntimeServiceId>,
                RuntimeServiceId,
            >(overwatch_handle.clone())
            .await
            .map_err(|error| {
                OperationStatus::error(
                    OperationStatusCode::RelayError,
                    format!("Failed to get current declaration id: {error}"),
                )
            })?
        else {
            return Ok(None);
        };

        let declarations = lb_api_service::http::mantle::get_sdp_declarations::<RuntimeServiceId>(
            &overwatch_handle,
        )
        .await
        .map_err(|error| {
            OperationStatus::error(
                OperationStatusCode::RelayError,
                format!("Failed to get SDP declarations: {error}"),
            )
        })?;

        Ok(declarations.get(&declaration_id).map(|declaration| {
            let created_epoch = declaration.created.into_inner();
            DeclarationInfo {
                id: declaration_id,
                provider_id: declaration.provider_id,
                zk_id: declaration.zk_id,
                locked_note_id: declaration.service_note_id,
                locators: declaration.locators.clone(),
                created_epoch,
                active_from_epoch: created_epoch
                    .saturating_add(sdp::SNAPSHOT_FINALIZATION_DELAY.into_inner()),
                withdraw_at: declaration.withdraw_at.map(lb_chain_service::Epoch::into_inner),
            }
        }))
    });
    let declaration = unwrap_or_return_error!(declaration);

    let status = BlendStatus {
        core_mode,
        declaration,
        current_epoch,
        session_peers,
    };
    FfiBlendStatusResult::ok(unwrap_or_return_error!(to_json_cstring(&status)))
}
