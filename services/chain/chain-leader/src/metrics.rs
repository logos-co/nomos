use std::time::Duration;

pub fn consensus_proposals_created_local() {
    lb_tracing::increase_counter_u64!(consensus_proposals_created_total, 1, origin = "local");
}

pub fn consensus_proposals_create_failed(reason: &'static str) {
    lb_tracing::increase_counter_u64!(consensus_proposals_create_failed_total, 1, reason = reason);
}

/// How long after its slot started a local proposal was handed to Blend, and
/// how much of that was leadership proof generation.
pub fn consensus_observe_proposal_published(since_slot_start_ms: i64, proof_duration: Duration) {
    lb_tracing::metric_histogram_f64!(
        consensus_proposal_publish_delay_seconds,
        since_slot_start_ms as f64 / 1000.0
    );
    lb_tracing::metric_histogram_f64!(consensus_leader_proof_seconds, proof_duration.as_secs_f64());
}
