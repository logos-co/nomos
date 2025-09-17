use std::num::NonZeroUsize;

use broadcast_service::{BlockBroadcastMsg, BlockInfo};
use cryptarchia_engine::{Branch, Cryptarchia, PrunedBlocks};
use cryptarchia_sync::HeaderId;
use overwatch::services::relay::RelayError;
use tokio::sync::broadcast;
use tracing::error;

use crate::{relays::BroadcastRelay, LibUpdate, PrunedBlocksInfo};

/// A trait for broadcasting finalized blocks to other services.
#[async_trait::async_trait]
pub trait BlockBroadcaster {
    async fn broadcast(
        &self,
        cryptarchia: &Cryptarchia<HeaderId>,
        pruned_blocks: &PrunedBlocks<HeaderId>,
        relay: &BroadcastRelay,
        lib_broadcaster: &broadcast::Sender<LibUpdate>,
    ) -> Result<(), Error>;
}

/// Creates a new [`BlockBroadcaster`] based on the state of the given
/// [`Cryptarchia`].
pub fn new(cryptarchia: &Cryptarchia<HeaderId>) -> Box<dyn BlockBroadcaster + Send> {
    if cryptarchia.state().is_bootstrapping() {
        Box::new(BootstrappingBlockBroadcaster::new(cryptarchia))
    } else {
        Box::new(OnlineBlockBroadcaster::new(cryptarchia))
    }
}

/// A [`BlockBroadcaster`] for [`Cryptarchia`] in Online state.
///
/// It broadcasts the new LIB when it changes.
struct OnlineBlockBroadcaster {
    prev_lib: HeaderId,
}

impl OnlineBlockBroadcaster {
    const fn new(cryptarchia: &Cryptarchia<HeaderId>) -> Self {
        assert!(cryptarchia.state().is_online());
        Self {
            prev_lib: cryptarchia.lib(),
        }
    }
}

#[async_trait::async_trait]
impl BlockBroadcaster for OnlineBlockBroadcaster {
    /// Broadcasts the current LIB, if it is different from the previous LIB.
    async fn broadcast(
        &self,
        cryptarchia: &Cryptarchia<HeaderId>,
        pruned_blocks: &PrunedBlocks<HeaderId>,
        relay: &BroadcastRelay,
        lib_broadcaster: &broadcast::Sender<LibUpdate>,
    ) -> Result<(), Error> {
        if !cryptarchia.state().is_online() {
            return Err(Error::UnexpectedCryptarchiaState(*cryptarchia.state()));
        }

        let lib = cryptarchia.lib_branch();
        if self.prev_lib != lib.id() {
            let lib_update = LibUpdate {
                new_lib: cryptarchia.lib(),
                pruned_blocks: PrunedBlocksInfo {
                    stale_blocks: pruned_blocks.stale_blocks().copied().collect(),
                    immutable_blocks: pruned_blocks.immutable_blocks().clone(),
                },
            };
            if let Err(e) = lib_broadcaster.send(lib_update) {
                error!("Could not notify LIB update to services: {e}");
            }

            broadcast_finalized_block(
                relay,
                BlockInfo {
                    height: lib.length(),
                    header_id: lib.id(),
                },
            )
            .await?;
        }

        Ok(())
    }
}

/// A [`BlockBroadcaster`] for [`Cryptarchia`] in Bootstrapping state.
///
/// The LIB never changes in Bootstrapping state.
/// So, this broadcasts potential-finalized blocks that are deeper than
/// the security parameter from the local chain tip.
struct BootstrappingBlockBroadcaster {
    prev_block_at_security_param: Branch<HeaderId>,
}

impl BootstrappingBlockBroadcaster {
    fn new(cryptarchia: &Cryptarchia<HeaderId>) -> Self {
        assert!(cryptarchia.state().is_bootstrapping());
        Self {
            prev_block_at_security_param: cryptarchia.block_at_security_param(),
        }
    }
}

#[async_trait::async_trait]
impl BlockBroadcaster for BootstrappingBlockBroadcaster {
    /// Broadcasts all blocks between the previous security block (exclusive)
    /// and the security block (inclusive).
    ///
    /// If the chain has been reorganized (i.e. the current security block is
    /// not a descedant of the previous one), all blocks between the common
    /// ancestor (exclusive) and the current security block (inclusive).
    async fn broadcast(
        &self,
        cryptarchia: &Cryptarchia<HeaderId>,
        _pruned_blocks: &PrunedBlocks<HeaderId>,
        relay: &BroadcastRelay,
        _lib_broadcaster: &broadcast::Sender<LibUpdate>,
    ) -> Result<(), Error> {
        if !cryptarchia.state().is_bootstrapping() {
            return Err(Error::UnexpectedCryptarchiaState(*cryptarchia.state()));
        }

        let new_block_at_security_param = cryptarchia.block_at_security_param();
        let lca = cryptarchia.branches().lca(
            &self.prev_block_at_security_param,
            &new_block_at_security_param,
        );
        let path = cryptarchia
            .blocks_in_inclusive_range(
                lca.id(),
                new_block_at_security_param.id(),
                NonZeroUsize::MAX,
            )?
            // Skip the first item, which is LCA.
            .skip(1);

        for branch in path {
            broadcast_finalized_block(
                relay,
                BlockInfo {
                    height: branch.length(),
                    header_id: branch.id(),
                },
            )
            .await?;
        }

        Ok(())
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Cryptarchia error: {0}")]
    Cryptarchia(#[from] cryptarchia_engine::Error<HeaderId>),
    #[error("Relay error: {0}")]
    Relay(#[from] RelayError),
    #[error("Unexpected Cryptarchia state: {0:?}")]
    UnexpectedCryptarchiaState(cryptarchia_engine::State),
}

async fn broadcast_finalized_block(
    broadcast_relay: &BroadcastRelay,
    block_info: BlockInfo,
) -> Result<(), RelayError> {
    broadcast_relay
        .send(BlockBroadcastMsg::BroadcastFinalizedBlock(block_info))
        .await
        .map_err(|(e, _)| e)
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use cryptarchia_engine::{Config, Slot, State};
    use tokio::sync::mpsc;

    use super::*;

    #[tokio::test]
    async fn online_block_broadcaster() {
        let config = Config {
            security_param: NonZeroU32::new(1).unwrap(),
            active_slot_coeff: 1.0,
        };

        // Init Cryptarchia with LIB 10.
        let cryptarchia = Cryptarchia::from_lib(id(10), config, State::Online);
        assert_eq!(cryptarchia.lib(), id(10));
        let broadcaster = new(&cryptarchia);

        // Add block 11 and 12.
        // Expect LIB to be 11, since security param is 1.
        let (cryptarchia, _) = cryptarchia
            .receive_block(id(11), id(10), Slot::from(1))
            .unwrap();
        let (cryptarchia, _) = cryptarchia
            .receive_block(id(12), id(11), Slot::from(2))
            .unwrap();
        assert_eq!(cryptarchia.lib(), id(11));

        // Broadcast blocks.
        let (relay, mut receiver) = relay();
        let (lib_broadcaster, _) = broadcast::channel(10);
        broadcaster
            .broadcast(
                &cryptarchia,
                &PrunedBlocks::default(),
                &relay,
                &lib_broadcaster,
            )
            .await
            .unwrap();

        // Expect only block 11 to be broadcasted.
        let BlockBroadcastMsg::BroadcastFinalizedBlock(BlockInfo { height, header_id }) =
            receiver.recv().await.unwrap()
        else {
            panic!("Unexpected message")
        };
        assert_eq!(height, 1);
        assert_eq!(header_id, id(11));

        assert!(receiver.is_empty());
    }

    #[tokio::test]
    async fn online_block_broadcaster_same_lib() {
        let config = Config {
            security_param: NonZeroU32::new(1).unwrap(),
            active_slot_coeff: 1.0,
        };

        // Init Cryptarchia with LIB 10.
        let cryptarchia = Cryptarchia::from_lib(id(10), config, State::Online);
        assert_eq!(cryptarchia.lib(), id(10));
        let broadcaster = new(&cryptarchia);

        // Add block 11.
        // LIB doesn't change, since security param is 1.
        let (cryptarchia, _) = cryptarchia
            .receive_block(id(11), id(10), Slot::from(1))
            .unwrap();
        assert_eq!(cryptarchia.lib(), id(10));

        // Broadcast blocks.
        let (relay, receiver) = relay();
        let (lib_broadcaster, _) = broadcast::channel(10);
        broadcaster
            .broadcast(
                &cryptarchia,
                &PrunedBlocks::default(),
                &relay,
                &lib_broadcaster,
            )
            .await
            .unwrap();

        // Expect no blocks to be broadcasted.
        assert!(receiver.is_empty());
    }

    #[tokio::test]
    async fn online_block_broadcaster_with_bootstrapping_state() {
        let config = Config {
            security_param: NonZeroU32::new(1).unwrap(),
            active_slot_coeff: 1.0,
        };

        // Init Cryptarchia with Online state.
        let cryptarchia = Cryptarchia::from_lib(id(10), config, State::Online);
        let broadcaster = new(&cryptarchia);

        // Broadcasting with Bootstrapping Cryptarchia should fail.
        let (relay, _) = relay();
        let (lib_broadcaster, _) = broadcast::channel(10);
        assert!(matches!(
            broadcaster
                .broadcast(
                    &Cryptarchia::from_lib(id(10), config, State::Bootstrapping),
                    &PrunedBlocks::default(),
                    &relay,
                    &lib_broadcaster,
                )
                .await,
            Err(Error::UnexpectedCryptarchiaState(State::Bootstrapping))
        ));
    }

    #[tokio::test]
    async fn bootstrapping_block_broadcaster() {
        let config = Config {
            security_param: NonZeroU32::new(1).unwrap(),
            active_slot_coeff: 1.0,
        };

        // Init Cryptarchia with LIB 10.
        let cryptarchia = Cryptarchia::from_lib(id(10), config, State::Bootstrapping);
        assert_eq!(cryptarchia.lib(), id(10));
        let broadcaster = new(&cryptarchia);

        // Add block 11 and 12.
        // Expect the security block to be 11, since security param is 1.
        let (cryptarchia, _) = cryptarchia
            .receive_block(id(11), id(10), Slot::from(1))
            .unwrap();
        let (cryptarchia, _) = cryptarchia
            .receive_block(id(12), id(11), Slot::from(2))
            .unwrap();
        assert_eq!(cryptarchia.block_at_security_param().id(), id(11));

        // Broadcast blocks.
        let (relay, mut receiver) = relay();
        let (lib_broadcaster, _) = broadcast::channel(10);
        broadcaster
            .broadcast(
                &cryptarchia,
                &PrunedBlocks::default(),
                &relay,
                &lib_broadcaster,
            )
            .await
            .unwrap();

        // Expect only block 11 to be broadcasted.
        let BlockBroadcastMsg::BroadcastFinalizedBlock(BlockInfo { height, header_id }) =
            receiver.recv().await.unwrap()
        else {
            panic!("Unexpected message")
        };
        assert_eq!(height, 1);
        assert_eq!(header_id, id(11));

        assert!(receiver.is_empty());
    }

    #[tokio::test]
    async fn bootstrapping_block_broadcaster_same_security_block() {
        let config = Config {
            security_param: NonZeroU32::new(1).unwrap(),
            active_slot_coeff: 1.0,
        };

        // Init Cryptarchia with LIB 10.
        let cryptarchia = Cryptarchia::from_lib(id(10), config, State::Bootstrapping);
        assert_eq!(cryptarchia.lib(), id(10));
        let broadcaster = new(&cryptarchia);

        // Add block 11.
        // The security block doesn't change, since security param is 1.
        let (cryptarchia, _) = cryptarchia
            .receive_block(id(11), id(10), Slot::from(1))
            .unwrap();
        assert_eq!(cryptarchia.block_at_security_param().id(), id(10));

        // Broadcast blocks.
        let (relay, receiver) = relay();
        let (lib_broadcaster, _) = broadcast::channel(10);
        broadcaster
            .broadcast(
                &cryptarchia,
                &PrunedBlocks::default(),
                &relay,
                &lib_broadcaster,
            )
            .await
            .unwrap();

        // Expect no blocks to be broadcasted.
        assert!(receiver.is_empty());
    }

    #[tokio::test]
    async fn bootstrapping_block_broadcaster_reorg() {
        let config = Config {
            security_param: NonZeroU32::new(1).unwrap(),
            active_slot_coeff: 1.0,
        };

        // Init Cryptarchia:
        //          k-th
        //           ||
        // 10 - 11 - 12 - 13
        let cryptarchia = Cryptarchia::from_lib(id(10), config, State::Bootstrapping);
        let (cryptarchia, _) = cryptarchia
            .receive_block(id(11), id(10), Slot::from(1))
            .unwrap();
        let (cryptarchia, _) = cryptarchia
            .receive_block(id(12), id(11), Slot::from(10))
            .unwrap();
        let (cryptarchia, _) = cryptarchia
            .receive_block(id(13), id(12), Slot::from(11))
            .unwrap();

        assert_eq!(cryptarchia.block_at_security_param().id(), id(12));
        let broadcaster = new(&cryptarchia);

        // Add a longer fork.
        // 10 - 11 - 12 - 13
        //         \
        //           14 - 15 - 16
        //                ||
        //               k-th
        let (cryptarchia, _) = cryptarchia
            .receive_block(id(14), id(11), Slot::from(4))
            .unwrap();
        let (cryptarchia, _) = cryptarchia
            .receive_block(id(15), id(14), Slot::from(5))
            .unwrap();
        let (cryptarchia, _) = cryptarchia
            .receive_block(id(16), id(15), Slot::from(6))
            .unwrap();
        assert_eq!(cryptarchia.block_at_security_param().id(), id(15));

        // Broadcast blocks.
        let (relay, mut receiver) = relay();
        let (lib_broadcaster, _) = broadcast::channel(10);
        broadcaster
            .broadcast(
                &cryptarchia,
                &PrunedBlocks::default(),
                &relay,
                &lib_broadcaster,
            )
            .await
            .unwrap();

        // Expect blocks 14 and 15 to be broadcasted.
        let BlockBroadcastMsg::BroadcastFinalizedBlock(BlockInfo { height, header_id }) =
            receiver.recv().await.unwrap()
        else {
            panic!("Unexpected message")
        };
        assert_eq!(height, 2);
        assert_eq!(header_id, id(14));
        let BlockBroadcastMsg::BroadcastFinalizedBlock(BlockInfo { height, header_id }) =
            receiver.recv().await.unwrap()
        else {
            panic!("Unexpected message")
        };
        assert_eq!(height, 3);
        assert_eq!(header_id, id(15));
        assert!(receiver.is_empty());
    }

    #[tokio::test]
    async fn bootstrapping_block_broadcaster_with_online_state() {
        let config = Config {
            security_param: NonZeroU32::new(1).unwrap(),
            active_slot_coeff: 1.0,
        };

        // Init Cryptarchia with Online state.
        let cryptarchia = Cryptarchia::from_lib(id(10), config, State::Bootstrapping);
        let broadcaster = new(&cryptarchia);

        // Broadcasting with Bootstrapping Cryptarchia should fail.
        let (relay, _) = relay();
        let (lib_broadcaster, _) = broadcast::channel(10);
        assert!(matches!(
            broadcaster
                .broadcast(
                    &Cryptarchia::from_lib(id(10), config, State::Online),
                    &PrunedBlocks::default(),
                    &relay,
                    &lib_broadcaster,
                )
                .await,
            Err(Error::UnexpectedCryptarchiaState(State::Online))
        ));
    }

    fn id(i: u8) -> HeaderId {
        HeaderId::from([i; 32])
    }

    fn relay() -> (BroadcastRelay, mpsc::Receiver<BlockBroadcastMsg>) {
        let (sender, receiver) = mpsc::channel(10);
        (BroadcastRelay::new(sender), receiver)
    }
}
