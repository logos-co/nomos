use cucumber::then;

use super::{error::StepResult, world::SqlWorld};

pub mod logos_sql;

const TARGET: &str = "cucumber_steps";

#[then("I stop all Logos SQL clients")]
async fn stop_clients(world: &mut SqlWorld) -> StepResult {
    world.logos_sql.shutdown_all().await
}
