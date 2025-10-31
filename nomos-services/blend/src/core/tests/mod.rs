mod utils;

use std::{sync::Arc, time::Duration};

use groth16::Field as _;
use nomos_blend_scheduling::message_blend::crypto::SessionCryptographicProcessorSettings;
use nomos_core::{crypto::ZkHash, mantle::keys::SecretKey};
use nomos_time::SlotTick;
use overwatch::{overwatch::OverwatchHandle, services::state::StateUpdater};
use poq::CORE_MERKLE_TREE_HEIGHT;
use tokio::{
    sync::{mpsc, watch},
    time::{sleep, timeout},
};

use crate::{
    core::{
        initialize, post_initialize,
        processor::tests::MockCoreAndLeaderProofsGenerator,
        run_event_loop,
        settings::{
            BlendConfig, CoverTrafficSettings, MessageDelayerSettings, SchedulerSettings,
            ZkSettings,
        },
        state::RecoveryServiceState,
        tests::utils::{NodeId, TestBlendBackend, TestNetworkAdapter, new_membership, new_stream},
    },
    epoch_info::EpochHandler,
    membership::{MembershipInfo, ZkInfo},
    settings::TimingSettings,
    test_utils::{
        crypto::MockProofsVerifier,
        epoch::{OncePolStreamProvider, TestChainService},
    },
};

type RuntimeServiceId = ();

#[test_log::test(tokio::test)]
async fn complete_old_session_after_main_loop_done() {
    let minimal_network_size = 2;
    let (membership, local_private_key) = new_membership(minimal_network_size);

    let recovery_file = tempfile::NamedTempFile::new().unwrap();
    let settings = BlendConfig {
        backend: (),
        crypto: SessionCryptographicProcessorSettings {
            non_ephemeral_signing_key: local_private_key,
            num_blend_layers: 1,
        },
        scheduler: SchedulerSettings {
            cover: CoverTrafficSettings {
                message_frequency_per_round: 1.0.try_into().unwrap(),
                redundancy_parameter: 0,
                intervals_for_safety_buffer: 0,
            },
            delayer: MessageDelayerSettings {
                maximum_release_delay_in_rounds: 1.try_into().unwrap(),
            },
        },
        time: TimingSettings {
            rounds_per_session: 10.try_into().unwrap(),
            rounds_per_interval: 5.try_into().unwrap(),
            round_duration: Duration::from_secs(1),
            rounds_per_observation_window: 5.try_into().unwrap(),
            rounds_per_session_transition_period: 2.try_into().unwrap(),
            epoch_transition_period_in_slots: 1.try_into().unwrap(),
        },
        zk: ZkSettings {
            sk: SecretKey::zero(),
        },
        minimum_network_size: u64::from(minimal_network_size).try_into().unwrap(),
        recovery_path: recovery_file.path().to_path_buf(),
    };

    let (overwatch_cmd_sender, _overwatch_cmd_receiver) = mpsc::channel(10);
    let overwatch_handle = OverwatchHandle::<RuntimeServiceId>::new(
        tokio::runtime::Handle::current(),
        overwatch_cmd_sender,
    );
    let (state_sender, _state_receiver) = watch::channel(None);
    let state_updater =
        StateUpdater::<Option<RecoveryServiceState<(), ()>>>::new(Arc::new(state_sender));

    let (inbound_relay, _inbound_message_sender) = new_stream();
    let (blend_message_stream, _blend_message_sender) = new_stream();

    let initial_session = 0;
    let mut membership_info = MembershipInfo {
        membership: membership.clone(),
        zk: ZkInfo {
            root: ZkHash::ZERO,
            core_and_path_selectors: Some([(ZkHash::ZERO, false); CORE_MERKLE_TREE_HEIGHT]),
        },
        session_number: initial_session,
    };
    let (membership_stream, membership_sender) = new_stream();
    membership_sender
        .send(membership_info.clone())
        .await
        .unwrap();

    let (clock_stream, clock_sender) = new_stream();
    clock_sender
        .send(SlotTick {
            epoch: 0.into(),
            slot: 0.into(),
        })
        .await
        .unwrap();

    let mut epoch_handler = EpochHandler::new(
        TestChainService,
        settings.time.epoch_transition_period_in_slots,
    );

    let (
        remaining_session_stream,
        remaining_clock_stream,
        current_public_info,
        crypto_processor,
        blending_token_collector,
        current_recovery_checkpoint,
        message_scheduler,
        backend,
        rng,
    ) = initialize::<
        NodeId,
        TestBlendBackend,
        TestNetworkAdapter,
        TestChainService,
        MockCoreAndLeaderProofsGenerator,
        MockProofsVerifier,
        RuntimeServiceId,
    >(
        settings.clone(),
        membership_stream,
        clock_stream,
        &mut epoch_handler,
        overwatch_handle.clone(),
        None,
        state_updater,
    )
    .await;

    let settings_cloned = settings.clone();
    let join_handle = tokio::spawn(async move {
        let secret_pol_info_stream =
            post_initialize::<OncePolStreamProvider, RuntimeServiceId>(&overwatch_handle).await;

        run_event_loop(
            inbound_relay,
            blend_message_stream,
            remaining_clock_stream,
            secret_pol_info_stream,
            remaining_session_stream,
            &settings_cloned,
            backend,
            TestNetworkAdapter,
            epoch_handler,
            message_scheduler,
            rng,
            blending_token_collector,
            crypto_processor,
            current_public_info,
            current_recovery_checkpoint,
        )
        .await;
    });

    // Send a new session with the same membership.
    membership_info.session_number += 1;
    membership_sender
        .send(membership_info.clone())
        .await
        .unwrap();

    // Since the node is still core in the new session,
    // the node must keep running even after a session transition period.
    sleep(settings.time.session_transition_period().mul_f64(1.5)).await;
    assert!(!join_handle.is_finished());

    // Send a new session with a new membership smaller than minimal size
    membership_info.membership = new_membership(minimal_network_size.checked_sub(1).unwrap()).0;
    membership_info.session_number += 1;
    membership_sender.send(membership_info).await.unwrap();

    // Since the network is smaller than the minimal size,
    // the node must stop after a session transition period.
    timeout(
        settings.time.session_transition_period().mul_f64(1.5),
        join_handle,
    )
    .await
    .expect("the node should stop")
    .expect("the node should stop without error");
}
