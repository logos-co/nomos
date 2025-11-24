use std::{collections::HashMap, time::Duration};

use async_trait::async_trait;
use rand::{Rng as _, seq::SliceRandom as _, thread_rng};
use testing_framework_core::scenario::{DynError, RunContext, Workload};
use tokio::time::{Instant, sleep};
use tracing::info;

pub struct RandomRestartWorkload {
    min_delay: Duration,
    max_delay: Duration,
    target_cooldown: Duration,
    include_validators: bool,
    include_executors: bool,
}

impl RandomRestartWorkload {
    #[must_use]
    pub const fn new(
        min_delay: Duration,
        max_delay: Duration,
        target_cooldown: Duration,
        include_validators: bool,
        include_executors: bool,
    ) -> Self {
        Self {
            min_delay,
            max_delay,
            target_cooldown,
            include_validators,
            include_executors,
        }
    }

    fn targets(&self, ctx: &RunContext) -> Vec<Target> {
        let mut targets = Vec::new();
        let validator_count = ctx.descriptors().validators().len();
        if self.include_validators {
            if validator_count > 1 {
                for index in 0..validator_count {
                    targets.push(Target::Validator(index));
                }
            } else if validator_count == 1 {
                info!("chaos restart skipping validators: only one validator configured");
            }
        }
        if self.include_executors {
            for index in 0..ctx.descriptors().executors().len() {
                targets.push(Target::Executor(index));
            }
        }
        targets
    }

    fn random_delay(&self) -> Duration {
        if self.max_delay <= self.min_delay {
            return self.min_delay;
        }
        let spread = self
            .max_delay
            .checked_sub(self.min_delay)
            .unwrap_or_else(|| Duration::from_millis(1))
            .as_secs_f64();
        let offset = thread_rng().gen_range(0.0..=spread);
        self.min_delay
            .checked_add(Duration::from_secs_f64(offset))
            .unwrap_or(self.max_delay)
    }

    fn initialize_cooldowns(&self, targets: &[Target]) -> HashMap<Target, Instant> {
        let now = Instant::now();
        let ready = now.checked_sub(self.target_cooldown).unwrap_or(now);
        targets
            .iter()
            .copied()
            .map(|target| (target, ready))
            .collect()
    }

    async fn pick_target(
        &self,
        targets: &[Target],
        cooldowns: &HashMap<Target, Instant>,
    ) -> Target {
        loop {
            let now = Instant::now();
            if let Some(next_ready) = cooldowns
                .values()
                .copied()
                .filter(|ready| *ready > now)
                .min()
            {
                let wait = next_ready.saturating_duration_since(now);
                if !wait.is_zero() {
                    sleep(wait).await;
                    continue;
                }
            }

            let available: Vec<Target> = targets
                .iter()
                .copied()
                .filter(|target| cooldowns.get(target).is_none_or(|ready| *ready <= now))
                .collect();

            if let Some(choice) = available.choose(&mut thread_rng()).copied() {
                return choice;
            }

            return targets
                .choose(&mut thread_rng())
                .copied()
                .expect("chaos restart workload has targets");
        }
    }
}

#[async_trait]
impl Workload for RandomRestartWorkload {
    fn name(&self) -> &'static str {
        "chaos_random_restart"
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        let handle = ctx
            .node_control()
            .ok_or_else(|| "chaos restart workload requires node control".to_owned())?;

        let targets = self.targets(ctx);
        if targets.is_empty() {
            return Err("chaos restart workload has no eligible targets".into());
        }

        let mut cooldowns = self.initialize_cooldowns(&targets);

        loop {
            sleep(self.random_delay()).await;
            let target = self.pick_target(&targets, &cooldowns).await;

            match target {
                Target::Validator(index) => handle
                    .restart_validator(index)
                    .await
                    .map_err(|err| format!("validator restart failed: {err}"))?,
                Target::Executor(index) => handle
                    .restart_executor(index)
                    .await
                    .map_err(|err| format!("executor restart failed: {err}"))?,
            }

            cooldowns.insert(target, Instant::now() + self.target_cooldown);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Target {
    Validator(usize),
    Executor(usize),
}
