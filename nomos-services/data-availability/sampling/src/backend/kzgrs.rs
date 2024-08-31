// std
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Debug;
use std::time::{Duration, Instant};

// crates
use rand::distributions::Standard;
use rand::prelude::*;
use tokio::time;
use tokio::time::Interval;

use kzgrs_backend::common::blob::DaBlob;

//
// internal
use super::DaSamplingServiceBackend;
use nomos_core::da::BlobId;
use nomos_da_network_core::SubnetworkId;

const NON_TRIGGERED_ERR: &str =
    "We should not receive a sampling success from a non triggered blobId";

#[derive(Clone)]
pub struct SamplingContext {
    subnets: HashSet<SubnetworkId>,
    started: Instant,
}

#[derive(Debug, Clone)]
pub struct KzgrsDaSamplerSettings {
    pub num_samples: u16,
    pub old_blobs_check_interval: Duration,
    pub blobs_validity_duration: Duration,
}

pub struct KzgrsDaSampler<R: Rng> {
    settings: KzgrsDaSamplerSettings,
    validated_blobs: BTreeSet<BlobId>,
    pending_sampling_blobs: HashMap<BlobId, SamplingContext>,
    rng: R,
}

impl<R: Rng> KzgrsDaSampler<R> {
    fn prune_by_time(&mut self) {
        self.pending_sampling_blobs.retain(|_blob_id, context| {
            context.started.elapsed() < self.settings.blobs_validity_duration
        });
    }
}

// TODO there is no logging at all here. Should we not do some logging?
// Or can we assume that it is done on an upeer level by the clients?
#[async_trait::async_trait]
impl<R: Rng + Sync + Send> DaSamplingServiceBackend<R> for KzgrsDaSampler<R> {
    type Settings = KzgrsDaSamplerSettings;
    type BlobId = BlobId;
    type Blob = DaBlob;

    fn new(settings: Self::Settings, rng: R) -> Self {
        let bt: BTreeSet<BlobId> = BTreeSet::new();
        Self {
            settings,
            validated_blobs: bt,
            pending_sampling_blobs: HashMap::new(),
            rng,
        }
    }

    async fn next_prune_interval(&self) -> Interval {
        time::interval(self.settings.old_blobs_check_interval)
    }

    async fn get_validated_blobs(&self) -> BTreeSet<Self::BlobId> {
        self.validated_blobs.clone()
    }

    async fn mark_in_block(&mut self, blobs_ids: &[Self::BlobId]) {
        for id in blobs_ids {
            self.pending_sampling_blobs.remove(id);
            self.validated_blobs.remove(id);
        }
    }

    async fn handle_sampling_success(&mut self, blob_id: Self::BlobId, blob: Self::Blob) {
        if let Some(ctx) = self.pending_sampling_blobs.get_mut(&blob_id) {
            ctx.subnets.insert(blob.column_idx as SubnetworkId);

            println!("{}", self.settings.num_samples);
            println!("{}", ctx.subnets.len());
            match ctx.subnets.len() {
                // sampling of this blob_id terminated successfully
                len if len == self.settings.num_samples as usize => {
                    self.validated_blobs.insert(blob_id);
                    // cleanup from pending samplings
                    self.pending_sampling_blobs.remove(&blob_id);
                }
                len if len > self.settings.num_samples as usize => {
                    unreachable!("{}", "more subnets than expected after sampling success!");
                }
                // do nothing if smaller
                _ => {}
            }
        } else {
            unreachable!("{}", NON_TRIGGERED_ERR);
        }
    }

    async fn handle_sampling_error(&mut self, blob_id: Self::BlobId) {
        // If it fails a single time we consider it failed.
        // We may want to abstract the sampling policies somewhere else at some point if we
        // need to get fancier than this
        self.pending_sampling_blobs.remove(&blob_id);
        self.validated_blobs.remove(&blob_id);
    }

    // TODO: This also would be an error which should never happen, but what if a client starts
    // init_sampling of a blob which is already pending? Or worse, which already is validated?
    // Should we not therefore return an error here?
    async fn init_sampling(&mut self, blob_id: Self::BlobId) -> Vec<SubnetworkId> {
        let subnets: Vec<SubnetworkId> = Standard
            .sample_iter(&mut self.rng)
            .take(self.settings.num_samples as usize)
            .collect();
        let ctx: SamplingContext = SamplingContext {
            subnets: HashSet::new(),
            started: Instant::now(),
        };
        self.pending_sampling_blobs.insert(blob_id, ctx);
        subnets
    }

    fn prune(&mut self) {
        self.prune_by_time()
    }
}

#[cfg(test)]
mod test {

    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    use rand::prelude::*;
    use rand::rngs::StdRng;

    use crate::backend::kzgrs::{
        DaSamplingServiceBackend, KzgrsDaSampler, KzgrsDaSamplerSettings, SamplingContext,
    };
    use kzgrs_backend::common::{blob::DaBlob, Column};
    use nomos_core::da::BlobId;
    use nomos_da_network_core::SubnetworkId;

    fn create_sampler(subnet_num: usize) -> KzgrsDaSampler<StdRng> {
        let settings = KzgrsDaSamplerSettings {
            num_samples: subnet_num as u16,
            old_blobs_check_interval: Duration::from_millis(20),
            blobs_validity_duration: Duration::from_millis(10),
        };
        let rng = StdRng::from_entropy();
        KzgrsDaSampler::new(settings, rng)
    }

    #[tokio::test]
    async fn test_sampler() {
        // fictitious number of subnets
        let subnet_num: usize = 42;

        // create a sampler instance
        let sampler = &mut create_sampler(subnet_num);

        // create some blobs and blob_ids
        let b1: BlobId = sampler.rng.gen();
        let b2: BlobId = sampler.rng.gen();
        let blob = DaBlob {
            column_idx: 42,
            column: Column(vec![]),
            column_commitment: Default::default(),
            aggregated_column_commitment: Default::default(),
            aggregated_column_proof: Default::default(),
            rows_commitments: vec![],
            rows_proofs: vec![],
        };
        let blob2 = blob.clone();
        let mut blob3 = blob2.clone();

        // at start everything should be empty
        assert!(sampler.pending_sampling_blobs.is_empty());
        assert!(sampler.validated_blobs.is_empty());
        assert!(sampler.get_validated_blobs().await.is_empty());

        // start sampling for b1
        let subnets_to_sample: Vec<SubnetworkId> = sampler.init_sampling(b1).await;
        assert!(subnets_to_sample.len() == subnet_num);
        assert!(sampler.validated_blobs.is_empty());
        assert!(sampler.pending_sampling_blobs.len() == 1);

        // start sampling for b2
        let subnets_to_sample2: Vec<SubnetworkId> = sampler.init_sampling(b2).await;
        assert!(subnets_to_sample2.len() == subnet_num);
        assert!(sampler.validated_blobs.is_empty());
        assert!(sampler.pending_sampling_blobs.len() == 2);

        // mark in block for both
        // collections should be reset
        sampler.mark_in_block(&[b1, b2]).await;
        assert!(sampler.pending_sampling_blobs.is_empty());
        assert!(sampler.validated_blobs.is_empty());

        // because they're reset, we need to restart sampling for the test
        _ = sampler.init_sampling(b1).await;
        _ = sampler.init_sampling(b2).await;

        // handle ficticious error for b2
        // b2 should be gone, b1 still around
        sampler.handle_sampling_error(b2).await;
        assert!(sampler.validated_blobs.is_empty());
        assert!(sampler.pending_sampling_blobs.len() == 1);
        assert!(sampler.pending_sampling_blobs.contains_key(&b1));

        // handle ficticious sampling success for b1
        // should still just have one pending blob, no validated blobs yet,
        // and one subnet added to blob
        sampler.handle_sampling_success(b1, blob).await;
        assert!(sampler.validated_blobs.is_empty());
        assert!(sampler.pending_sampling_blobs.len() == 1);
        println!(
            "{}",
            sampler
                .pending_sampling_blobs
                .get(&b1)
                .unwrap()
                .subnets
                .len()
        );
        assert!(
            sampler
                .pending_sampling_blobs
                .get(&b1)
                .unwrap()
                .subnets
                .len()
                == 1
        );

        // handle_success for always the same subnet
        // by adding number of subnets time the same subnet
        // (column_idx did not change)
        // should not change, still just one sampling blob,
        // no validated blobs and one subnet
        for _ in 1..subnet_num {
            let b = blob2.clone();
            sampler.handle_sampling_success(b1, b).await;
        }
        assert!(sampler.validated_blobs.is_empty());
        assert!(sampler.pending_sampling_blobs.len() == 1);
        assert!(
            sampler
                .pending_sampling_blobs
                .get(&b1)
                .unwrap()
                .subnets
                .len()
                == 1
        );

        // handle_success for up to subnet size minus one subnet
        // should still not change anything
        // but subnets len is now subnet size minus one
        // we already added subnet 42
        for i in 1..(subnet_num - 1) {
            let mut b = blob2.clone();
            b.column_idx = i as u16;
            sampler.handle_sampling_success(b1, b).await;
        }
        assert!(sampler.validated_blobs.is_empty());
        assert!(sampler.pending_sampling_blobs.len() == 1);
        assert!(
            sampler
                .pending_sampling_blobs
                .get(&b1)
                .unwrap()
                .subnets
                .len()
                == subnet_num - 1
        );

        // now add the last subnet!
        // we should have all subnets set,
        // and the validated blobs should now have that blob
        // pending blobs should now be empty
        blob3.column_idx = (subnet_num - 1) as u16;
        sampler.handle_sampling_success(b1, blob3).await;
        assert!(sampler.pending_sampling_blobs.is_empty());
        assert!(sampler.validated_blobs.len() == 1);
        assert!(sampler.validated_blobs.contains(&b1));
        // these checks are redundant but better safe than sorry
        assert!(sampler.get_validated_blobs().await.len() == 1);
        assert!(sampler.get_validated_blobs().await.contains(&b1));

        // run mark_in_block for the same blob
        // should return empty for everything
        sampler.mark_in_block(&[b1]).await;
        assert!(sampler.validated_blobs.is_empty());
        assert!(sampler.pending_sampling_blobs.is_empty());
    }

    #[tokio::test]
    async fn test_panic() {
        // a bit of an overkill for a test but adds coverage:
        // tests that trying to handle success for a blob
        // which hasn't been initated for sampling first,
        // should panic
        use futures::FutureExt;
        use std::panic::AssertUnwindSafe;

        let mut sampler = create_sampler(42);

        let b: BlobId = sampler.rng.gen();
        let blob = DaBlob {
            column_idx: 42,
            column: Column(vec![]),
            column_commitment: Default::default(),
            aggregated_column_commitment: Default::default(),
            aggregated_column_proof: Default::default(),
            rows_commitments: vec![],
            rows_proofs: vec![],
        };

        let should_panic = async {
            AssertUnwindSafe(sampler.handle_sampling_success(b, blob)).await;
            // needed to avoid "may not be safely transferred across an unwind boundary" errors
            drop(sampler);
        };
        let should_panic_result = should_panic.catch_unwind().await;
        assert!(should_panic_result.is_err());
    }

    #[tokio::test]
    async fn test_pruning() {
        let mut sampler = create_sampler(42);

        // create some sampling contexes
        // first set will go through as in time
        let ctx1 = SamplingContext {
            subnets: HashSet::new(),
            started: Instant::now(),
        };
        let ctx2 = ctx1.clone();
        let ctx3 = ctx1.clone();

        // second set: will fail for expired
        let ctx11 = SamplingContext {
            subnets: HashSet::new(),
            started: Instant::now() - Duration::from_secs(1),
        };
        let ctx12 = ctx11.clone();
        let ctx13 = ctx11.clone();

        // create a couple blob ids
        let b1: BlobId = sampler.rng.gen();
        let b2: BlobId = sampler.rng.gen();
        let b3: BlobId = sampler.rng.gen();

        // insert first blob
        // pruning should have no effect
        assert!(sampler.pending_sampling_blobs.is_empty());
        sampler.pending_sampling_blobs.insert(b1, ctx1);
        sampler.prune();
        assert!(sampler.pending_sampling_blobs.len() == 1);

        // insert second blob
        // pruning should have no effect
        sampler.pending_sampling_blobs.insert(b2, ctx2);
        sampler.prune();
        assert!(sampler.pending_sampling_blobs.len() == 2);

        // insert third blob
        // pruning should have no effect
        sampler.pending_sampling_blobs.insert(b3, ctx3);
        sampler.prune();
        assert!(sampler.pending_sampling_blobs.len() == 3);

        // insert fake expired blobs
        // pruning these should now decrease pending blobx every time
        sampler.pending_sampling_blobs.insert(b1, ctx11);
        sampler.prune();
        assert!(sampler.pending_sampling_blobs.len() == 2);

        sampler.pending_sampling_blobs.insert(b2, ctx12);
        sampler.prune();
        assert!(sampler.pending_sampling_blobs.len() == 1);

        sampler.pending_sampling_blobs.insert(b3, ctx13);
        sampler.prune();
        assert!(sampler.pending_sampling_blobs.is_empty());
    }
}
