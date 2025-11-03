mod utils;

use std::time::Duration;

use groth16::Field as _;
use nomos_core::crypto::ZkHash;
use nomos_time::SlotTick;
use poq::CORE_MERKLE_TREE_HEIGHT;

use crate::{
    core::{
        initialize, post_initialize, run_event_loop,
        tests::utils::{
            NodeId, TestBlendBackend, TestBlendBackendEvent, TestNetworkAdapter,
            dummy_overwatch_resources, new_membership, new_stream, settings,
            wait_for_blend_backend_event,
        },
    },
    epoch_info::EpochHandler,
    membership::{MembershipInfo, ZkInfo},
    settings::TimingSettings,
    test_utils::{
        crypto::{MockCoreAndLeaderProofsGenerator, MockProofsVerifier},
        epoch::{OncePolStreamProvider, TestChainService},
    },
};

type RuntimeServiceId = ();

/// Check if the service keeps running after it receives a new session where
/// it's still core. Also, check if it stops after the session transition period
/// if it receives another new session that doesn't meet the core node
/// conditions.
#[test_log::test(tokio::test)]
async fn complete_old_session_after_main_loop_done() {
    let minimal_network_size = 2;
    let (membership, local_private_key) = new_membership(minimal_network_size);

    // Create settings.
    let (settings, _recovery_file) = settings(
        local_private_key.clone(),
        u64::from(minimal_network_size).try_into().unwrap(),
        (),
        TimingSettings {
            rounds_per_session: 10.try_into().unwrap(),
            rounds_per_interval: 5.try_into().unwrap(),
            round_duration: Duration::from_secs(1),
            rounds_per_observation_window: 5.try_into().unwrap(),
            rounds_per_session_transition_period: 2.try_into().unwrap(),
            epoch_transition_period_in_slots: 1.try_into().unwrap(),
        },
    );

    // Prepare streams.
    let (inbound_relay, _inbound_message_sender) = new_stream();
    let (blend_message_stream, _blend_message_sender) = new_stream();
    let (membership_stream, membership_sender) = new_stream();
    let (clock_stream, clock_sender) = new_stream();

    // Send the initial membership info that the service will expect to receive
    // immediately.
    let initial_session = 0;
    let mut membership_info = MembershipInfo {
        membership: membership.clone(),
        zk: ZkInfo {
            root: ZkHash::ZERO,
            core_and_path_selectors: Some([(ZkHash::ZERO, false); CORE_MERKLE_TREE_HEIGHT]),
        },
        session_number: initial_session,
    };
    membership_sender
        .send(membership_info.clone())
        .await
        .unwrap();

    // Send the initial slot tick that the service will expect to receive
    // immediately.
    clock_sender
        .send(SlotTick {
            epoch: 0.into(),
            slot: 0.into(),
        })
        .await
        .unwrap();

    // Prepare an epoch handler with the mock chain service that always returns the
    // same epoch state.
    let mut epoch_handler = EpochHandler::new(
        TestChainService,
        settings.time.epoch_transition_period_in_slots,
    );

    // Prepare dummy Overwatch resources.
    let (overwatch_handle, _overwatch_cmd_receiver, state_updater, _state_receiver) =
        dummy_overwatch_resources();

    // Initialize the service.
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
    let mut backend_event_receiver = backend.subscribe_to_events();

    // Run the event loop of the service in a separate task.
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
    // the service must keep running even after a session transition period.
    wait_for_blend_backend_event(
        &mut backend_event_receiver,
        TestBlendBackendEvent::SessionTransitionCompleted,
    )
    .await;
    assert!(!join_handle.is_finished());

    // Send a new session with a new membership smaller than minimal size
    membership_info.membership = new_membership(minimal_network_size.checked_sub(1).unwrap()).0;
    membership_info.session_number += 1;
    membership_sender.send(membership_info).await.unwrap();

    // Since the network is smaller than the minimal size,
    // the service must stop after a session transition period.
    wait_for_blend_backend_event(
        &mut backend_event_receiver,
        TestBlendBackendEvent::SessionTransitionCompleted,
    )
    .await;
    join_handle
        .await
        .expect("the service should stop without error");
}
