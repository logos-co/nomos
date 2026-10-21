use std::sync::LazyLock;

use cucumber::{
    World as _,
    codegen::LocalBoxFuture,
    step::{Collection, Context},
};
use logos_blockchain_tests::cucumber::{
    shared_steps::{register_steps, with_node_steps},
    world::CucumberWorld,
};

use super::world::ZoneWorld;

static ZONE_STEPS: LazyLock<Collection<ZoneWorld>> = LazyLock::new(ZoneWorld::collection);

// SQL scenarios reuse ZoneSDK steps, but their callbacks expect ZoneWorld,
// not SqlWorld. Look up the original callback and its parsed arguments, then
// pass it the embedded ZoneWorld returned by AsMut.
fn forward_zone<W: AsMut<ZoneWorld>>(world: &mut W, context: Context) -> LocalBoxFuture<'_, ()> {
    Box::pin(async move {
        let (step, _, _, context) = ZONE_STEPS
            .find(&context.step)
            .expect("zone step must be unambiguous")
            .expect("zone step was registered");

        step(world.as_mut(), context).await;
    })
}

/// Adds both node and `ZoneSDK` steps to a suite, such as the SQL scenarios.
pub fn with_zone_steps<W: AsMut<CucumberWorld> + AsMut<ZoneWorld>>(
    steps: Collection<W>,
) -> Collection<W> {
    register_steps::<W, ZoneWorld>(with_node_steps(steps), forward_zone)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use cucumber::{
        World as _,
        gherkin::{Feature, GherkinEnv},
    };
    use logos_blockchain_tests::cucumber::world::DeployerKind;

    use super::{ZoneWorld, with_node_steps};

    #[test]
    fn all_zone_scenario_steps_are_registered() {
        let steps = with_node_steps(ZoneWorld::collection());
        let features = Path::new(env!("CARGO_MANIFEST_DIR")).join("features");

        for entry in fs::read_dir(features).expect("feature directory exists") {
            let path = entry.expect("feature entry is readable").path();
            let feature = Feature::parse_path(&path, GherkinEnv::default()).expect("valid feature");
            let scenarios = feature
                .scenarios
                .iter()
                .chain(feature.rules.iter().flat_map(|rule| &rule.scenarios));

            for scenario in scenarios {
                for step in &scenario.steps {
                    assert!(
                        steps.find(step).expect("step is unambiguous").is_some(),
                        "{}: {}: undefined step {step}",
                        path.display(),
                        scenario.name,
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn node_steps_update_embedded_node_state() {
        let feature = Feature::parse(
            "Feature: Shared node setup\n  Scenario: Configure deployment\n    Given deployer is \"local\"\n",
            GherkinEnv::default(),
        ).expect("valid feature");
        let steps = with_node_steps(ZoneWorld::collection());
        let (step, _, _, context) = steps
            .find(&feature.scenarios[0].steps[0])
            .expect("unambiguous step")
            .expect("registered node step");
        let mut world = ZoneWorld::default();

        step(&mut world, context).await;

        assert_eq!(world.node.lifecycle.deployer, Some(DeployerKind::Local));
    }
}
