mod async_runner;
mod glauber_runner;
mod layered_runner;
mod sync_runner;

// std
use std::marker::PhantomData;
use std::sync::{Arc, RwLock};
use std::time::Duration;

// crates
use crate::streaming::{Producer, Subscriber};
use crossbeam::channel::Sender;
use rand::rngs::SmallRng;
use rand::{RngCore, SeedableRng};
use rayon::prelude::*;
use serde::Serialize;

// internal
use crate::network::Network;
use crate::node::Node;
use crate::overlay::Overlay;
use crate::settings::{RunnerSettings, SimulationSettings};
use crate::streaming::StreamSettings;
use crate::warding::{SimulationState, SimulationWard, Ward};

pub struct SimulationRunnerHandle {
    handle: std::thread::JoinHandle<anyhow::Result<()>>,
    stop_tx: Sender<()>,
}

impl SimulationRunnerHandle {
    pub fn stop_after(self, duration: Duration) -> anyhow::Result<()> {
        std::thread::sleep(duration);
        self.stop()
    }

    pub fn stop(self) -> anyhow::Result<()> {
        if !self.handle.is_finished() {
            self.stop_tx.send(())?;
        }
        Ok(())
    }
}

pub(crate) struct SimulationRunnerInner<M> {
    network: Network<M>,
    wards: Vec<Ward>,
    rng: SmallRng,
}

impl<M> SimulationRunnerInner<M>
where
    M: Send + Sync + Clone,
{
    fn check_wards<N>(&mut self, state: &SimulationState<N>) -> bool
    where
        N: Node + Send + Sync,
        N::Settings: Clone + Send,
        N::State: Serialize,
    {
        self.wards
            .par_iter_mut()
            .map(|ward| ward.analyze(state))
            .any(|x| x)
    }

    fn step<N>(&mut self, nodes: &mut Vec<N>)
    where
        N: Node + Send + Sync,
        N::Settings: Clone + Send,
        N::State: Serialize,
    {
        self.network.dispatch_after(Duration::from_millis(100));
        nodes.par_iter_mut().for_each(|node| {
            node.step();
        });
        self.network.collect_messages();
    }
}

/// Encapsulation solution for the simulations runner
/// Holds the network state, the simulating nodes and the simulation settings.
pub struct SimulationRunner<M, N, O, P>
where
    N: Node,
    O: Overlay,
    P: Producer,
{
    inner: Arc<RwLock<SimulationRunnerInner<M>>>,
    nodes: Arc<RwLock<Vec<N>>>,
    runner_settings: RunnerSettings,
    stream_settings: StreamSettings<P::Settings>,
    _overlay: PhantomData<O>,
}

impl<M, N: Node, O: Overlay, P: Producer> SimulationRunner<M, N, O, P>
where
    M: Clone + Send + Sync + 'static,
    N: Send + Sync + 'static,
    N::Settings: Clone + Send,
    N::State: Serialize,
    O::Settings: Clone + Send,
    P::Subscriber: Send + Sync + 'static,
    <P::Subscriber as Subscriber>::Record:
        Send + Sync + 'static + for<'a> TryFrom<&'a SimulationState<N>, Error = anyhow::Error>,
{
    pub fn new(
        network: Network<M>,
        nodes: Vec<N>,
        settings: SimulationSettings<N::Settings, O::Settings, P::Settings>,
    ) -> Self {
        let seed = settings
            .seed
            .unwrap_or_else(|| rand::thread_rng().next_u64());

        println!("Seed: {seed}");

        let rng = SmallRng::seed_from_u64(seed);
        let nodes = Arc::new(RwLock::new(nodes));
        let SimulationSettings {
            network_behaviors: _,
            regions: _,
            wards,
            overlay_settings: _,
            node_settings: _,
            runner_settings,
            stream_settings,
            node_count: _,
            committee_size: _,
            seed: _,
        } = settings;
        Self {
            stream_settings,
            runner_settings,
            inner: Arc::new(RwLock::new(SimulationRunnerInner {
                network,
                rng,
                wards,
            })),
            nodes,
            _overlay: PhantomData,
        }
    }

    pub fn simulate(self) -> anyhow::Result<SimulationRunnerHandle> {
        match self.runner_settings.clone() {
            RunnerSettings::Sync => sync_runner::simulate::<_, _, _, P>(self),
            RunnerSettings::Async { chunks } => async_runner::simulate::<_, _, _, P>(self, chunks),
            RunnerSettings::Glauber {
                maximum_iterations,
                update_rate,
            } => glauber_runner::simulate::<_, _, _, P>(self, update_rate, maximum_iterations),
            RunnerSettings::Layered {
                rounds_gap,
                distribution,
            } => layered_runner::simulate::<_, _, _, P>(self, rounds_gap, distribution),
        }
    }
}
