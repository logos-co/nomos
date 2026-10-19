use cucumber::then;

use super::world::ZoneWorld;

pub mod zone;

const TARGET: &str = "cucumber_steps";

#[then("I stop all ZoneSDK clients")]
fn stop_clients(world: &mut ZoneWorld) {
    world.zone.clear();
}
