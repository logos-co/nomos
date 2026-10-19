use std::{path::Path, process::ExitCode};

use cucumber::World as _;
use logos_blockchain_tests::cucumber::{
    deployment::LocalImplementation, runner, shared_steps::with_node_steps,
};

use self::world::ZoneWorld;

pub mod shared_steps;
pub mod steps;
pub mod world;
mod zone;

#[expect(
    clippy::future_not_send,
    reason = "Cucumber uses non-Send futures; the entry point awaits the runner directly"
)]
pub async fn run() -> ExitCode {
    runner::run_suite::<ZoneWorld>(
        LocalImplementation::Logos,
        Path::new(env!("CARGO_MANIFEST_DIR")).join("features"),
        with_node_steps(ZoneWorld::collection()),
        ZoneWorld::full_debug_info_string,
    )
    .await
}
