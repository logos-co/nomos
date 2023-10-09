use crate::backend::{DaBackend, DaError};
use moka::future::{Cache, CacheBuilder};
use nomos_core::da::blob::Blob;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct BlobCacheSettings {
    pub max_capacity: usize,
    pub evicting_period: Duration,
}

pub struct BlobCache<H, B>(Cache<H, B>);

impl<B> BlobCache<B::Hash, B>
where
    B: Clone + Blob + Send + Sync + 'static,
    B::Hash: Send + Sync + 'static,
{
    pub fn new(settings: BlobCacheSettings) -> Self {
        let BlobCacheSettings {
            max_capacity,
            evicting_period,
        } = settings;
        let cache = CacheBuilder::new(max_capacity as u64)
            .time_to_live(evicting_period)
            // can we leverage this to evict really old blobs?
            .time_to_idle(evicting_period)
            .build();
        Self(cache)
    }

    pub async fn add(&self, blob: B) {
        self.0.insert(blob.hash(), blob).await
    }

    pub async fn remove(&self, hash: &B::Hash) {
        self.0.remove(hash).await;
    }
}

#[async_trait::async_trait]
impl<B> DaBackend for BlobCache<B::Hash, B>
where
    B: Clone + Blob + Send + Sync + 'static,
    B::Hash: Send + Sync + 'static,
{
    type Settings = BlobCacheSettings;
    type Blob = B;

    fn new(settings: Self::Settings) -> Self {
        BlobCache::new(settings)
    }

    async fn add_blob(&self, blob: Self::Blob) -> Result<(), DaError> {
        self.add(blob).await;
        Ok(())
    }

    async fn remove_blob(&self, blob: &<Self::Blob as Blob>::Hash) -> Result<(), DaError> {
        self.remove(blob).await;
        Ok(())
    }

    fn get_blob(&self, id: &<Self::Blob as Blob>::Hash) -> Option<Self::Blob> {
        self.0.get(id)
    }
}
