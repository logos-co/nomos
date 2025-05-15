use std::{
    collections::HashSet,
    hash::Hash,
    marker::PhantomData,
    ops::Div as _,
    pin::Pin,
    task::{Context, Poll},
};

use blake2::{digest::consts::U4, Digest as _};
use futures::{Stream, StreamExt as _};
use nomos_blend_message::BlendMessage;
use serde::Deserialize;

#[derive(Copy, Clone, Deserialize)]
pub struct CoverTrafficSettings {
    pub node_id: [u8; 32],
    pub number_of_hops: usize,
    pub slots_per_epoch: usize,
    pub network_size: usize,
}

pub struct CoverTraffic<EpochStream, SlotStream, Message> {
    winning_probability: f64,
    settings: CoverTrafficSettings,
    epoch_stream: EpochStream,
    slot_stream: SlotStream,
    selected_slots: HashSet<u32>,
    _message: PhantomData<Message>,
}

impl<EpochStream, SlotStream, Message> CoverTraffic<EpochStream, SlotStream, Message>
where
    EpochStream: Stream<Item = usize> + Send + Sync + Unpin,
    SlotStream: Stream<Item = usize> + Send + Sync + Unpin,
{
    pub fn new(
        settings: CoverTrafficSettings,
        epoch_stream: EpochStream,
        slot_stream: SlotStream,
    ) -> Self {
        let winning_probability = winning_probability(settings.number_of_hops);
        Self {
            winning_probability,
            settings,
            epoch_stream,
            slot_stream,
            selected_slots: HashSet::default(),
            _message: PhantomData,
        }
    }
}

impl<EpochStream, SlotStream, Message> Stream for CoverTraffic<EpochStream, SlotStream, Message>
where
    EpochStream: Stream<Item = usize> + Send + Sync + Unpin,
    SlotStream: Stream<Item = usize> + Send + Sync + Unpin,
    Message: BlendMessage + Send + Sync + Unpin,
{
    type Item = Vec<u8>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let Self {
            winning_probability,
            settings,
            epoch_stream,
            slot_stream,
            selected_slots,
            ..
        } = &mut *self;
        if let Poll::Ready(Some(epoch)) = epoch_stream.poll_next_unpin(cx) {
            *selected_slots = select_slot(
                settings.node_id,
                epoch,
                settings.network_size,
                settings.slots_per_epoch,
                *winning_probability,
            );
        }
        if let Poll::Ready(Some(slot)) = slot_stream.poll_next_unpin(cx) {
            if selected_slots.contains(&(slot as u32)) {
                return Poll::Ready(Some(vec![]));
            }
        }
        Poll::Pending
    }
}

fn generate_ticket<Id: Hash + Eq + AsRef<[u8]>>(node_id: Id, r: usize, slot: usize) -> u32 {
    let mut hasher = blake2::Blake2s::<U4>::new();
    hasher.update(node_id);
    hasher.update(r.to_be_bytes());
    hasher.update(slot.to_be_bytes());
    let hash: [u8; std::mem::size_of::<u32>()] = hasher.finalize()[..].to_vec().try_into().unwrap();
    u32::from_be_bytes(hash)
}

fn select_slot<Id: Hash + Eq + AsRef<[u8]> + Copy>(
    node_id: Id,
    r: usize,
    network_size: usize,
    slots_per_epoch: usize,
    winning_probability: f64,
) -> HashSet<u32> {
    let i = (slots_per_epoch as f64).div(network_size as f64) * winning_probability;
    let size = i.ceil() as usize;
    let mut w = HashSet::new();
    let mut i = 0;
    while w.len() != size {
        w.insert(generate_ticket(node_id, r, i) % slots_per_epoch as u32);
        i += 1;
    }
    w
}

fn winning_probability(number_of_hops: usize) -> f64 {
    1.0 / number_of_hops as f64
}

#[cfg(test)]
mod tests {
    use crate::cover_traffic::{generate_ticket, select_slot, winning_probability};

    #[test]
    fn test_ticket() {
        generate_ticket(10u32.to_be_bytes(), 1123, 0);
        for i in 0..1u32 {
            let slots = select_slot(i.to_be_bytes(), 1234, 100, 21600, winning_probability(1));
            println!("slots = {slots:?}");
        }
    }
}
