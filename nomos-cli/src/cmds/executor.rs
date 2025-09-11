use std::{path::PathBuf, sync::mpsc::Sender};

use clap::Args;
use executor_http_client::{BasicAuthCredentials, ExecutorHttpClient};
use kzgrs_backend::{dispersal::Metadata, encoder::DaEncoderParams};
use nomos_core::{da::BlobId, mantle::ops::channel::ChannelId};
use reqwest::Url;

#[derive(Args, Debug)]
pub struct Disseminate {
    #[clap(short, long)]
    pub channel_id: String,
    /// Text to disseminate.
    #[clap(short, long, required_unless_present("file"))]
    pub data: Option<String>,
    /// File to disseminate.
    #[clap(short, long)]
    pub file: Option<PathBuf>,
    /// Application ID for dispersed data.
    #[clap(long)]
    pub app_id: String,
    /// Index for the Blob associated with Application ID.
    #[clap(long)]
    pub index: u64,
    /// Executor address which is responsible for dissemination.
    #[clap(long)]
    pub addr: Url,
    /// Optional username for authentication.
    #[clap(long)]
    pub username: Option<String>,
    /// Optional password for authentication.
    #[clap(long)]
    pub password: Option<String>,
}

impl Disseminate {
    #[expect(
        clippy::cognitive_complexity,
        reason = "TODO: Address this at some point."
    )]
    pub fn run(self) -> Result<(), Box<dyn std::error::Error>> {
        let basic_auth = self
            .username
            .map(|u| BasicAuthCredentials::new(u, self.password.clone()));

        let client = ExecutorHttpClient::new(basic_auth);

        let mut bytes: Vec<u8> = if let Some(data) = &self.data {
            data.clone().into_bytes()
        } else {
            let file_path = self.file.as_ref().unwrap();
            std::fs::read(file_path)?
        };

        let remainder = bytes.len() % DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE;
        if remainder != 0 {
            bytes.resize(
                bytes.len() + (DaEncoderParams::MAX_BLS12_381_ENCODING_CHUNK_SIZE - remainder),
                0,
            );
        }

        let channel_id: [u8; 32] = hex::decode(&self.channel_id)?
            .try_into()
            .map_err(|_| "Invalid app_id")?;
        let app_id: [u8; 32] = hex::decode(&self.app_id)?
            .try_into()
            .map_err(|_| "Invalid app_id")?;
        let metadata = Metadata::new(app_id, self.index.into());

        let (res_sender, res_receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            disperse_data(
                &res_sender,
                &client,
                self.addr.clone(),
                channel_id.into(),
                bytes,
                metadata,
            );
        });

        match res_receiver.recv() {
            Ok(update) => match update {
                Ok(_) => tracing::info!("Data successfully disseminated."),
                Err(e) => {
                    tracing::error!("Error disseminating data: {e}");
                    return Err(e.into());
                }
            },
            Err(e) => {
                tracing::error!("Failed to receive from client thread: {e}");
                return Err(e.into());
            }
        }

        tracing::info!("Done");
        Ok(())
    }
}

#[tokio::main]
async fn disperse_data(
    res_sender: &Sender<Result<BlobId, String>>,
    client: &ExecutorHttpClient,
    base_url: Url,
    channel_id: ChannelId,
    bytes: Vec<u8>,
    metadata: Metadata,
) {
    let res = client
        .publish_blob(base_url, channel_id, bytes, metadata)
        .await
        .map_err(|err| format!("Failed to publish blob: {err:?}"));
    res_sender.send(res).unwrap();
}
