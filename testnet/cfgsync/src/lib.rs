pub mod client;
pub mod config;
pub mod repo;
pub mod server;

#[cfg(test)]
mod tests {
    use std::{
        net::{Ipv4Addr, SocketAddr},
        num::NonZero,
        str::FromStr,
        time::Duration,
    };

    use futures::future::join_all;
    use nomos_da_dispersal::backend::kzgrs::MempoolPublishStrategy;
    use nomos_da_network_core::swarm::ReplicationConfig;
    use nomos_libp2p::{ed25519, libp2p, Multiaddr, PeerId, Protocol};
    use nomos_node::Config as ValidatorConfig;
    use nomos_tracing_service::TracingSettings;
    use subnetworks_assignations::MembershipHandler;
    use tokio::time::timeout;

    use crate::{
        client::get_config,
        server::{cfgsync_app, CfgSyncConfig},
    };

    #[tokio::test]
    async fn test_address_book() {
        let n_hosts = 4;
        let config = CfgSyncConfig {
            n_hosts,
            timeout: 10,
            port: 16,
            security_param: NonZero::new(1).unwrap(),
            active_slot_coeff: 0.0,
            subnetwork_size: 0,
            dispersal_factor: 0,
            num_samples: 0,
            num_subnets: 0,
            old_blobs_check_interval: Duration::ZERO,
            blobs_validity_duration: Duration::ZERO,
            global_params_path: String::new(),
            mempool_publish_strategy: MempoolPublishStrategy::Immediately,
            min_dispersal_peers: 0,
            min_replication_peers: 0,
            monitor_failure_time_window: Duration::from_nanos(1),
            balancer_interval: Duration::ZERO,
            tracing_settings: TracingSettings::default(),
            replication_settings: ReplicationConfig {
                seen_message_cache_size: 0,
                seen_message_ttl: Duration::from_secs(60),
            },
        };

        let app_addr: SocketAddr = "127.0.0.1:4321".parse().unwrap();
        let app = cfgsync_app(config.into());

        let (tx, app_ready) = tokio::sync::oneshot::channel();
        let _server_task = tokio::spawn(async move {
            tx.send(()).unwrap();
            axum::Server::bind(&app_addr)
                .serve(app.into_make_service())
                .await
                .unwrap();
        });

        assert!(timeout(Duration::from_millis(50), app_ready).await.is_ok());

        let client_fn = |i| async move {
            let ip = Ipv4Addr::from_str(&format!("1.0.0.{i}")).unwrap();
            (
                ip,
                get_config::<ValidatorConfig>(
                    ip,
                    ip.to_string(),
                    &format!("http://{app_addr}/validator"),
                )
                .await,
            )
        };

        let tasks: Vec<_> = (0..n_hosts).map(client_fn).collect();
        let results = join_all(tasks).await;

        for (my_ip, config) in results {
            assert_eq_da_membership(my_ip, &config.unwrap());
        }
    }

    pub fn assert_eq_da_membership(my_ip: Ipv4Addr, config: &ValidatorConfig) {
        let key = libp2p::identity::Keypair::from(ed25519::Keypair::from(
            config.da_network.backend.node_key.clone(),
        ));
        let my_peer_id = PeerId::from_public_key(&key.public());
        let my_multiaddr = config
            .da_network
            .backend
            .membership
            .get_address(&my_peer_id)
            .unwrap();
        let my_multiaddr_ip = extract_ip(&my_multiaddr).unwrap();
        assert_eq!(
            my_ip, my_multiaddr_ip,
            "DA membership ip doesn't match host ip"
        );
    }

    pub fn extract_ip(multiaddr: &Multiaddr) -> Option<Ipv4Addr> {
        for protocol in multiaddr {
            if let Protocol::Ip4(ip) = protocol {
                return Some(ip);
            }
        }
        None
    }
}
