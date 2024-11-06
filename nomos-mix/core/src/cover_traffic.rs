use blake2::Digest;
use futures::{Stream, StreamExt};
use nomos_mix_message::MixMessage;
use rand::{Rng, RngCore};
use std::collections::HashSet;
use std::hash::Hash;
use std::marker::PhantomData;
use std::ops::DerefMut;
use std::pin::Pin;
use std::task::{Context, Poll};

pub struct CoverTrafficSettings {
    node_id: [u8; 32],
    number_of_hops: usize,
    slots_per_epoch: usize,
    network_size: usize,
}
pub struct CoverTraffic<EpochStream, SlotStream, Message, Rng> {
    winning_probability: f64,
    current_probability: f64,
    settings: CoverTrafficSettings,
    epoch_stream: EpochStream,
    slot_stream: SlotStream,
    selected_slots: HashSet<u32>,
    rng: Rng,
    _message: PhantomData<Message>,
}

impl<EpochStream, SlotStream, Message, Rng> CoverTraffic<EpochStream, SlotStream, Message, Rng>
where
    EpochStream: Stream<Item = usize>,
    SlotStream: Stream<Item = usize>,
    Rng: RngCore,
{
    pub fn new(
        settings: CoverTrafficSettings,
        epoch_stream: EpochStream,
        slot_stream: SlotStream,
        rng: Rng,
    ) -> Self {
        let winning_probability = winning_probability(settings.number_of_hops);
        CoverTraffic {
            winning_probability,
            current_probability: winning_probability,
            settings,
            epoch_stream,
            slot_stream,
            selected_slots: Default::default(),
            rng,
            _message: Default::default(),
        }
    }
}

impl<EpochStream, SlotStream, Message, Rng> Stream
    for CoverTraffic<EpochStream, SlotStream, Message, Rng>
where
    EpochStream: Stream<Item = usize> + Unpin,
    SlotStream: Stream<Item = usize> + Unpin,
    Message: MixMessage + Unpin,
    Rng: RngCore + Unpin,
{
    type Item = Vec<u8>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Self {
            winning_probability,
            current_probability,
            settings,
            epoch_stream,
            slot_stream,
            selected_slots,
            rng,
            ..
        } = self.deref_mut();
        if let Poll::Ready(Some(epoch)) = epoch_stream.poll_next_unpin(cx) {
            *selected_slots = select_slot(
                settings.node_id,
                epoch,
                settings.network_size,
                settings.slots_per_epoch,
            );
        }
        if let Poll::Ready(Some(slot)) = slot_stream.poll_next_unpin(cx) {
            if selected_slots.contains(&(slot as u32)) {
                if coin_flip(*winning_probability, rng) {
                    *current_probability = *winning_probability;
                    return Poll::Ready(Some(vec![]));
                } else {
                    *current_probability += *winning_probability;
                }
            }
        }
        Poll::Pending
    }
}

fn generate_ticket<Id: Hash + Eq + AsRef<[u8]>>(node_id: Id, r: usize, slot: usize) -> u32 {
    let mut hasher = blake2::Blake2b512::new();
    hasher.update(node_id);
    hasher.update(r.to_be_bytes());
    hasher.update(slot.to_be_bytes());
    let hash = &hasher.finalize()[..];
    u32::from_be_bytes(hash.try_into().unwrap())
}

fn select_slot<Id: Hash + Eq + AsRef<[u8]> + Copy>(
    node_id: Id,
    r: usize,
    network_size: usize,
    slots_per_epoch: usize,
) -> HashSet<u32> {
    let i = network_size.div_ceil(slots_per_epoch);
    (0..i)
        .map(|i| generate_ticket(node_id, r, i) % slots_per_epoch as u32)
        .collect()
}

fn winning_probability(number_of_hops: usize) -> f64 {
    1.0 / number_of_hops as f64
}

fn coin_flip<Rng: RngCore>(probability: f64, rng: &mut Rng) -> bool {
    probability >= rng.gen_range(0.0..1.0)
}
