use std::fmt::{Debug, Display};

use nomos_network::{
    backends::libp2p::{Command, Libp2p, Libp2pInfo, NetworkCommand::Info},
    message::NetworkMsg,
    NetworkService,
};
use overwatch::services::AsServiceId;
use tokio::sync::oneshot;

pub async fn libp2p_info<RuntimeServiceId>(
    handle: &overwatch::overwatch::handle::OverwatchHandle<RuntimeServiceId>,
) -> Result<Libp2pInfo, overwatch::DynError>
where
    RuntimeServiceId:
        AsServiceId<NetworkService<Libp2p, RuntimeServiceId>> + Debug + Sync + Display + 'static,
{
    let relay = handle.relay().await?;
    let (sender, receiver) = oneshot::channel();

    relay
        .send(NetworkMsg::Process(Command::Network(Info {
            reply: sender,
        })))
        .await
        .map_err(|(e, _)| e)?;

    receiver
        .await
        .map_err(|e| Box::new(e) as overwatch::DynError)
}
