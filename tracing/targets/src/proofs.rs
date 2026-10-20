use lb_log_targets_macros::log_targets;

log_targets! {
    root = proofs;

    LEADER,
    LEADER_CLAIM,
    // TODO(RFC-480): LEADER_CLAIM, the vouchers and the PoC are retired.
    // POC,
    POL,
    POQ,
    PROGRAM_SPEND,
    // TODO(RFC-480): the ZkSignature circuit is retired (circuits v0.6.0).
    // ZKSIGN,
    ZKTRANSFER,
}
