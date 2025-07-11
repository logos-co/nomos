use std::{hash::Hash, time::Duration};

use nomos_utils::bounded_duration::{MinimalBoundedDuration, SECOND};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::bootstrap::{ibd::IbdConfig, initialization::InitializationConfig};

#[serde_with::serde_as]
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BootstrapConfig<PeerId>
where
    PeerId: Clone + Eq + Hash,
{
    pub initialization: InitializationConfig,
    pub ibd: IbdConfig<PeerId>,
    #[serde_as(as = "MinimalBoundedDuration<0, SECOND>")]
    pub prolonged_bootstrap_period: Duration,
}
