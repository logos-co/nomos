#![allow(clippy::allow_attributes_without_reason)]
#![allow(clippy::undocumented_unsafe_blocks)]

mod api;

use std::ffi::c_char;

pub use api::{NomosNode, stop_node};
use nomos_node::{
    AdapterSettings, Config, Nomos, NomosServiceSettings, SignedMantleTx,
    SignedTxProcessorSettings, Transaction, TxMempoolSettings, get_services_to_start,
};
use overwatch::overwatch::OverwatchRunner;
use tokio::runtime::Runtime;

#[repr(C)]
pub enum NomosNodeErrorCode {
    None = 0x0,
    CouldNotInitialize = 0x1,
    StopError,
    NullPtr,
}

#[repr(C)]
pub struct InitializedNomosNodeResult {
    nomos_node: *mut NomosNode,
    success: bool,
    error_code: NomosNodeErrorCode,
}

#[unsafe(no_mangle)]
pub extern "C" fn start_nomos_node(config_path: *const c_char) -> InitializedNomosNodeResult {
    match initialize_nomos_node(config_path) {
        Ok(nomos_node) => {
            let node_ptr = Box::into_raw(Box::new(nomos_node));
            InitializedNomosNodeResult {
                nomos_node: node_ptr,
                success: false,
                error_code: NomosNodeErrorCode::None,
            }
        }
        Err(error_code) => InitializedNomosNodeResult {
            nomos_node: core::ptr::null_mut(),
            success: false,
            error_code,
        },
    }
}

fn initialize_nomos_node(config_path: *const c_char) -> Result<NomosNode, NomosNodeErrorCode> {
    let must_blend_service_group_start = true;
    let must_da_service_group_start = true;
    let config_path = unsafe { std::ffi::CStr::from_ptr(config_path) }
        .to_str()
        .map_err(|_| NomosNodeErrorCode::CouldNotInitialize)?;
    let config = serde_yaml::from_reader::<_, Config>(
        std::fs::File::open(config_path).map_err(|_| NomosNodeErrorCode::CouldNotInitialize)?,
    )
    .map_err(|_| NomosNodeErrorCode::CouldNotInitialize)?;

    let (blend_config, blend_core_config, blend_edge_config) = config.blend.into();
    let rt = Runtime::new().unwrap();
    let handle = rt.handle();
    let app = OverwatchRunner::<Nomos>::run(
        NomosServiceSettings {
            network: config.network,
            blend: blend_config,
            blend_core: blend_core_config,
            blend_edge: blend_edge_config,
            block_broadcast: (),
            tracing: config.tracing,
            http: config.http,
            cl_mempool: TxMempoolSettings {
                pool: (),
                network_adapter: AdapterSettings {
                    topic: String::from(nomos_node::CL_TOPIC),
                    id: <SignedMantleTx as Transaction>::hash,
                },
                processor: SignedTxProcessorSettings {
                    trigger_sampling_delay: config.mempool.trigger_sampling_delay,
                },
                recovery_path: config.mempool.cl_pool_recovery_path,
            },
            da_network: config.da_network,
            da_sampling: config.da_sampling,
            da_verifier: config.da_verifier,
            cryptarchia: config.cryptarchia,
            time: config.time,
            storage: config.storage,
            system_sig: (),
            sdp: (),
            membership: config.membership,
        },
        Some(handle.clone()),
    )
    .map_err(|_| NomosNodeErrorCode::CouldNotInitialize)?;

    let app_handel = app.handle();

    rt.block_on(async {
        let services_to_start = get_services_to_start(
            &app,
            must_blend_service_group_start,
            must_da_service_group_start,
        )
        .await
        .map_err(|_| NomosNodeErrorCode::CouldNotInitialize)?;
        let _ = app_handel.start_service_sequence(services_to_start).await;
        Ok(())
    })?;

    Ok(NomosNode::new(app, rt))
}
