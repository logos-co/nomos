use std::{path::Path, process::ExitCode};

use cucumber::World as _;
use logos_blockchain_tests::cucumber::{deployment::LocalImplementation, runner};
use logos_blockchain_zone_tests::cucumber::shared_steps::with_zone_steps;

pub mod error;
pub mod logos_sql;
pub mod steps;
pub mod world;

use world::SqlWorld;

#[expect(
    clippy::future_not_send,
    reason = "Cucumber uses non-Send futures; the entry point awaits the runner directly"
)]
pub async fn run() -> ExitCode {
    runner::run_suite::<SqlWorld>(
        LocalImplementation::Logos,
        Path::new(env!("CARGO_MANIFEST_DIR")).join("features"),
        with_zone_steps(SqlWorld::collection()),
        SqlWorld::full_debug_info_string,
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::fs;

    use cucumber::gherkin::{Feature, GherkinEnv};

    use super::*;

    #[test]
    fn all_sql_scenario_steps_are_registered() {
        let steps = with_zone_steps(SqlWorld::collection());
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
}
