use std::{num::NonZeroUsize, time::Duration};

use libp2p::{
    gossipsub, identify,
    identity::{self, ed25519},
    kad, StreamProtocol,
};
use serde::{Deserialize, Serialize};

use crate::protocol_name::ProtocolName;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmConfig {
    // Listening IPv4 address
    pub host: std::net::Ipv4Addr,
    // TCP listening port. Use 0 for random
    pub port: u16,
    // Secp256k1 private key in Hex format (`0x123...abc`). Default random
    #[serde(with = "secret_key_serde", default = "ed25519::SecretKey::generate")]
    pub node_key: ed25519::SecretKey,
    // Gossipsub config
    #[serde(with = "GossipsubConfigDef", default = "gossipsub::Config::default")]
    pub gossipsub_config: gossipsub::Config,

    /// Protocol name env for Kademlia and Identify protocol names.
    /// This is used to determine the protocol names for Kademlia and Identify.
    ///
    /// Allowed values are:
    /// - `mainnet`
    /// - `testnet`
    /// - `unittest`
    /// - `integration`
    ///
    /// Default is `unittest`.
    #[serde(default)]
    pub protocol_name_env: ProtocolName,

    /// Kademlia config
    /// When a value is None, kademlia is disabled.
    /// Note: Kademlia requires identify or another identity protocol to be
    /// enabled.
    #[serde(default)]
    pub kademlia_config: Option<KademliaSettings>,

    /// Identify config
    /// When a value is None, identify is disabled.
    #[serde(default)]
    pub identify_config: Option<IdentifySettings>,
}

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            host: std::net::Ipv4Addr::new(0, 0, 0, 0),
            port: 60000,
            node_key: ed25519::SecretKey::generate(),
            gossipsub_config: gossipsub::Config::default(),
            protocol_name_env: ProtocolName::default(),
            kademlia_config: None,
            identify_config: None,
        }
    }
}

// A partial copy of gossipsub::Config for deriving Serialize/Deserialize
// remotely https://serde.rs/remote-derive.html
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(remote = "gossipsub::Config")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "Type matching `gossipsub::Config` for remote serde impl."
)]
struct GossipsubConfigDef {
    #[serde(getter = "gossipsub::Config::history_length")]
    history_length: usize,
    #[serde(getter = "gossipsub::Config::history_gossip")]
    history_gossip: usize,
    #[serde(getter = "gossipsub::Config::mesh_n")]
    mesh_n: usize,
    #[serde(getter = "gossipsub::Config::mesh_n_low")]
    mesh_n_low: usize,
    #[serde(getter = "gossipsub::Config::mesh_n_high")]
    mesh_n_high: usize,
    #[serde(getter = "gossipsub::Config::retain_scores")]
    retain_scores: usize,
    #[serde(getter = "gossipsub::Config::gossip_lazy")]
    gossip_lazy: usize,
    #[serde(getter = "gossipsub::Config::gossip_factor")]
    gossip_factor: f64,
    #[serde(getter = "gossipsub::Config::heartbeat_initial_delay")]
    heartbeat_initial_delay: Duration,
    #[serde(getter = "gossipsub::Config::heartbeat_interval")]
    heartbeat_interval: Duration,
    #[serde(getter = "gossipsub::Config::fanout_ttl")]
    fanout_ttl: Duration,
    #[serde(getter = "gossipsub::Config::check_explicit_peers_ticks")]
    check_explicit_peers_ticks: u64,
    #[serde(getter = "gossipsub::Config::duplicate_cache_time")]
    duplicate_cache_time: Duration,
    #[serde(getter = "gossipsub::Config::validate_messages")]
    validate_messages: bool,
    #[serde(getter = "gossipsub::Config::allow_self_origin")]
    allow_self_origin: bool,
    #[serde(getter = "gossipsub::Config::do_px")]
    do_px: bool,
    #[serde(getter = "gossipsub::Config::prune_peers")]
    prune_peers: usize,
    #[serde(getter = "gossipsub::Config::prune_backoff")]
    prune_backoff: Duration,
    #[serde(getter = "gossipsub::Config::unsubscribe_backoff")]
    unsubscribe_backoff: Duration,
    #[serde(getter = "gossipsub::Config::backoff_slack")]
    backoff_slack: u32,
    #[serde(getter = "gossipsub::Config::flood_publish")]
    flood_publish: bool,
    #[serde(getter = "gossipsub::Config::graft_flood_threshold")]
    graft_flood_threshold: Duration,
    #[serde(getter = "gossipsub::Config::mesh_outbound_min")]
    mesh_outbound_min: usize,
    #[serde(getter = "gossipsub::Config::opportunistic_graft_ticks")]
    opportunistic_graft_ticks: u64,
    #[serde(getter = "gossipsub::Config::opportunistic_graft_peers")]
    opportunistic_graft_peers: usize,
    #[serde(getter = "gossipsub::Config::gossip_retransimission")]
    gossip_retransimission: u32,
    #[serde(getter = "gossipsub::Config::max_messages_per_rpc")]
    max_messages_per_rpc: Option<usize>,
    #[serde(getter = "gossipsub::Config::max_ihave_length")]
    max_ihave_length: usize,
    #[serde(getter = "gossipsub::Config::max_ihave_messages")]
    max_ihave_messages: usize,
    #[serde(getter = "gossipsub::Config::iwant_followup_time")]
    iwant_followup_time: Duration,
    #[serde(getter = "gossipsub::Config::published_message_ids_cache_time")]
    published_message_ids_cache_time: Duration,
}

#[expect(
    clippy::fallible_impl_from,
    reason = "`TryFrom` impl conflicting with blanket impl."
)]
impl From<GossipsubConfigDef> for gossipsub::Config {
    fn from(def: GossipsubConfigDef) -> Self {
        let mut builder = gossipsub::ConfigBuilder::default();
        let mut builder = builder
            .history_length(def.history_length)
            .history_gossip(def.history_gossip)
            .mesh_n(def.mesh_n)
            .mesh_n_low(def.mesh_n_low)
            .mesh_n_high(def.mesh_n_high)
            .retain_scores(def.retain_scores)
            .gossip_lazy(def.gossip_lazy)
            .gossip_factor(def.gossip_factor)
            .heartbeat_initial_delay(def.heartbeat_initial_delay)
            .heartbeat_interval(def.heartbeat_interval)
            .fanout_ttl(def.fanout_ttl)
            .check_explicit_peers_ticks(def.check_explicit_peers_ticks)
            .duplicate_cache_time(def.duplicate_cache_time)
            .allow_self_origin(def.allow_self_origin)
            .prune_peers(def.prune_peers)
            .prune_backoff(def.prune_backoff)
            .unsubscribe_backoff(def.unsubscribe_backoff.as_secs())
            .backoff_slack(def.backoff_slack)
            .flood_publish(def.flood_publish)
            .graft_flood_threshold(def.graft_flood_threshold)
            .mesh_outbound_min(def.mesh_outbound_min)
            .opportunistic_graft_ticks(def.opportunistic_graft_ticks)
            .opportunistic_graft_peers(def.opportunistic_graft_peers)
            .gossip_retransimission(def.gossip_retransimission)
            .max_messages_per_rpc(def.max_messages_per_rpc)
            .max_ihave_length(def.max_ihave_length)
            .max_ihave_messages(def.max_ihave_messages)
            .iwant_followup_time(def.iwant_followup_time)
            .published_message_ids_cache_time(def.published_message_ids_cache_time);

        if def.validate_messages {
            builder = builder.validate_messages();
        }
        if def.do_px {
            builder = builder.do_px();
        }

        builder.build().unwrap()
    }
}

pub mod secret_key_serde {
    use libp2p::identity::ed25519;
    use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(key: &ed25519::SecretKey, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let hex_str = hex::encode(key.as_ref());
        hex_str.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ed25519::SecretKey, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex_str = String::deserialize(deserializer)?;
        let mut key_bytes = hex::decode(hex_str).map_err(|e| D::Error::custom(format!("{e}")))?;
        ed25519::SecretKey::try_from_bytes(key_bytes.as_mut_slice())
            .map_err(|e| D::Error::custom(format!("{e}")))
    }
}

/// A serializable representation of Kademlia configuration options.
/// When a value is None, the libp2p defaults are used.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KademliaSettings {
    /// The timeout for a single query in seconds
    /// Default from libp2p: 60 seconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_timeout_secs: Option<u64>,

    /// The replication factor to use
    /// Default from libp2p: 20 (`K_VALUE`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replication_factor: Option<NonZeroUsize>,

    /// The allowed level of parallelism for iterative queries
    /// Default from libp2p: 3 (`ALPHA_VALUE`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallelism: Option<NonZeroUsize>,

    /// Require iterative queries to use disjoint paths
    /// Default from libp2p: false
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disjoint_query_paths: Option<bool>,

    /// Maximum allowed size of individual Kademlia packets
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_packet_size: Option<usize>,

    /// The k-bucket insertion strategy
    /// Default from libp2p: "`on_connected`"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kbucket_inserts: Option<KBucketInserts>,

    /// The caching strategy
    /// Default from libp2p: Enabled with `max_peers=1`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caching: Option<CachingSettings>,

    /// The interval in seconds for periodic bootstrap
    /// If enabled the periodic bootstrap will run every x seconds in addition
    /// to the automatic bootstrap that is triggered when a new peer is added
    /// Default from libp2p: 5 minutes (300 seconds)
    /// None means use libp2p default
    /// Some(0) means periodic bootstrap is disabled
    /// Some(x) means periodic bootstrap is enabled for x seconds period
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub periodic_bootstrap_interval_secs: Option<u64>,

    /// The Kademlia node is in client mode if it does not
    /// expose its own Kademlia ID and only connects to other nodes
    /// Default from libp2p: false (server mode)
    #[serde(default)]
    pub client_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordFiltering {
    Unfiltered,
    FilterBoth,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KBucketInserts {
    OnConnected,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "config")]
pub enum CachingSettings {
    Disabled,
    Enabled { max_peers: u16 },
}

impl KademliaSettings {
    #[must_use]
    pub fn to_libp2p_config(&self, protocol_name: ProtocolName) -> kad::Config {
        let mut config = kad::Config::new(StreamProtocol::new(protocol_name.kad_protocol_name()));

        if let Some(timeout) = self.query_timeout_secs {
            config.set_query_timeout(Duration::from_secs(timeout));
        }

        if let Some(replication_factor) = self.replication_factor {
            config.set_replication_factor(replication_factor);
        }

        if let Some(parallelism) = self.parallelism {
            config.set_parallelism(parallelism);
        }

        if let Some(disjoint) = self.disjoint_query_paths {
            config.disjoint_query_paths(disjoint);
        }

        if let Some(size) = self.max_packet_size {
            config.set_max_packet_size(size);
        }

        if let Some(inserts) = &self.kbucket_inserts {
            match inserts {
                KBucketInserts::OnConnected => {
                    config.set_kbucket_inserts(kad::BucketInserts::OnConnected);
                }
                KBucketInserts::Manual => {
                    config.set_kbucket_inserts(kad::BucketInserts::Manual);
                }
            }
        }

        if let Some(caching) = &self.caching {
            match caching {
                CachingSettings::Disabled => {
                    config.set_caching(kad::Caching::Disabled);
                }
                CachingSettings::Enabled { max_peers } => {
                    config.set_caching(kad::Caching::Enabled {
                        max_peers: *max_peers,
                    });
                }
            }
        }

        // Set the periodic bootstrap interval if provided
        // If not provided, use the libp2p default
        // disable periodic bootstrap if interval is 0
        if let Some(interval) = &self.periodic_bootstrap_interval_secs {
            if *interval == 0 {
                config.set_periodic_bootstrap_interval(None);
            } else {
                config.set_periodic_bootstrap_interval(Some(Duration::from_secs(*interval)));
            }
        }

        config
    }
}

/// A serializable representation of Identify configuration options.
/// When a value is None, the libp2p defaults are used.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdentifySettings {
    /// Agent version string to advertise
    /// Default from libp2p: 'rust-libp2p/{version}'
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,

    /// Interval in seconds between pushes of identify info
    /// Default from libp2p: 5 minutes (300 seconds)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u64>,

    /// Whether new/expired listen addresses should trigger
    /// an active push of an identify message to all connected peers
    /// Default from libp2p: false
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_listen_addr_updates: Option<bool>,

    /// How many entries of discovered peers to keep
    /// Default from libp2p: 100
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_size: Option<usize>,

    /// Whether to hide listen addresses in responses (only share external
    /// addresses) Default from libp2p: false
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hide_listen_addrs: Option<bool>,
}

impl IdentifySettings {
    #[must_use]
    pub fn to_libp2p_config(
        &self,
        public_key: identity::PublicKey,
        protocol_name: ProtocolName,
    ) -> identify::Config {
        let mut config = identify::Config::new(
            protocol_name.identify_protocol_name().to_owned(),
            public_key,
        );

        // Apply only the settings that were specified, leaving libp2p defaults for the
        // rest
        if let Some(agent_version) = &self.agent_version {
            config = config.with_agent_version(agent_version.clone());
        }

        if let Some(interval) = self.interval_secs {
            config = config.with_interval(Duration::from_secs(interval));
        }

        if let Some(push_updates) = self.push_listen_addr_updates {
            config = config.with_push_listen_addr_updates(push_updates);
        }

        if let Some(cache_size) = self.cache_size {
            config = config.with_cache_size(cache_size);
        }

        if let Some(hide_addrs) = self.hide_listen_addrs {
            config = config.with_hide_listen_addrs(hide_addrs);
        }

        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_serde() {
        let config = SwarmConfig::default();

        let serialized = serde_json::to_string(&config).unwrap();
        println!("{serialized}");

        let deserialized: SwarmConfig = serde_json::from_str(serialized.as_str()).unwrap();
        assert_eq!(deserialized.host, config.host);
        assert_eq!(deserialized.port, config.port);
        assert_eq!(deserialized.node_key.as_ref(), config.node_key.as_ref());
    }
}
