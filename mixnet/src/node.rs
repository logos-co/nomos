use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::fragment::{Fragment, MessageReconstructor};
use crate::packet::{Message, Packet, PacketBody};
use crate::{crypto::PrivateKey, error::MixnetError, poisson::Poisson};

/// Mix node implementation that returns [`Output`] if exists.
pub struct MixNode {
    output_rx: mpsc::UnboundedReceiver<Output>,
}

struct MixNodeRunner {
    config: MixNodeConfig,
    poisson: Poisson,
    packet_queue: mpsc::Receiver<Box<[u8]>>,
    message_reconstructor: MessageReconstructor,
    output_tx: mpsc::UnboundedSender<Output>,
}

/// Mix node configuration
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct MixNodeConfig {
    /// Private key for decrypting Sphinx packets
    pub encryption_private_key: PrivateKey,
    /// Poisson delay rate per minutes
    pub delay_rate_per_min: f64,
}

const PACKET_QUEUE_SIZE: usize = 256;

/// Queue for sending packets to [`MixNode`]
pub type PacketQueue = mpsc::Sender<Box<[u8]>>;

impl MixNode {
    /// Creates a [`MixNode`] and a [`PacketQueue`].
    ///
    /// This returns [`MixnetError`] if the given `config` is invalid.
    pub fn new(config: MixNodeConfig) -> Result<(Self, PacketQueue), MixnetError> {
        let poisson = Poisson::new(config.delay_rate_per_min)?;
        let (packet_tx, packet_rx) = mpsc::channel(PACKET_QUEUE_SIZE);
        let (output_tx, output_rx) = mpsc::unbounded_channel();

        MixNodeRunner {
            config,
            poisson,
            packet_queue: packet_rx,
            message_reconstructor: MessageReconstructor::new(),
            output_tx,
        }
        .run();

        Ok((Self { output_rx }, packet_tx))
    }

    /// Returns a next `[Output]` to be emitted, if it exists and the Poisson delay is done (if necessary).
    pub async fn next(&mut self) -> Option<Output> {
        self.output_rx.recv().await
    }
}

impl MixNodeRunner {
    fn run(mut self) {
        tokio::spawn(async move {
            loop {
                if let Some(packet) = self.packet_queue.recv().await {
                    if let Err(e) = self.process_packet(packet.as_ref()) {
                        tracing::error!("failed to process packet. skipping it: {e}");
                    }
                }
            }
        });
    }

    fn process_packet(&mut self, packet: &[u8]) -> Result<(), MixnetError> {
        match PacketBody::from_bytes(packet)? {
            PacketBody::SphinxPacket(packet) => self.process_sphinx_packet(packet.as_ref())?,
            PacketBody::Fragment(fragment) => self.process_fragment(fragment.as_ref())?,
        }
        Ok(())
    }

    fn process_sphinx_packet(&self, packet: &[u8]) -> Result<(), MixnetError> {
        let output = Output::Forward(PacketBody::process_sphinx_packet(
            packet,
            &self.config.encryption_private_key,
        )?);
        let delay = self.poisson.interval(&mut OsRng);
        let output_tx = self.output_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            // output_tx is always expected to be not closed/dropped.
            output_tx.send(output).unwrap();
        });
        Ok(())
    }

    fn process_fragment(&mut self, fragment: &[u8]) -> Result<(), MixnetError> {
        if let Some(msg) = self
            .message_reconstructor
            .add(Fragment::from_bytes(fragment)?)
        {
            match Message::from_bytes(&msg)? {
                Message::Real(msg) => {
                    let output = Output::ReconstructedMessage(msg.to_vec().into_boxed_slice());
                    // output_tx is always expected to be not closed/dropped.
                    self.output_tx.send(output).unwrap();
                }
                Message::DropCover(_) => {
                    tracing::debug!("Drop cover message has been reconstructed. Dropping it...");
                }
            }
        }
        Ok(())
    }
}

/// Output that [`MixNode::next`] returns.
#[derive(Debug, PartialEq, Eq)]
pub enum Output {
    /// Packet to be forwarded to the next mix node
    Forward(Packet),
    /// Message reconstructed from [`Packet`]s
    ReconstructedMessage(Box<[u8]>),
}
