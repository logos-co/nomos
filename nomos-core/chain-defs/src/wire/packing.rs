use std::{io, mem::size_of};

use futures::{AsyncReadExt, AsyncWriteExt};
use serde::{de::DeserializeOwned, Serialize};
use thiserror::Error;

use crate::wire;

type Result<T> = std::result::Result<T, PackingError>;

type LenType = u16;
const MAX_MSG_LEN_BYTES: usize = size_of::<LenType>();
const MAX_MSG_LEN: usize = LenType::MAX as usize;

#[derive(Debug, Error)]
pub enum PackingError {
    #[error("Message too large. Maximum size is {max}, actual size is {actual}")]
    MessageTooLarge { max: usize, actual: usize },

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    #[error("Serialization error")]
    Serialization(#[from] wire::Error),

    #[error("Length conversion failed")]
    LengthConversion,
}

const fn get_packed_message_size(packed_message: &[u8]) -> Result<usize> {
    let data_length = packed_message.len();
    if data_length > MAX_MSG_LEN {
        return Err(PackingError::MessageTooLarge {
            max: MAX_MSG_LEN,
            actual: data_length,
        });
    }
    Ok(data_length)
}

pub async fn pack_to_writer<Message, Writer>(message: &Message, writer: &mut Writer) -> Result<()>
where
    Message: Serialize + Sync,
    Writer: AsyncWriteExt + Send + Unpin,
{
    let packed_message = wire::serialize(message)?;
    let data_length = get_packed_message_size(&packed_message)?;

    let length_prefix: LenType = data_length
        .try_into()
        .map_err(|_| PackingError::LengthConversion)?;

    writer
        .write_all(&length_prefix.to_be_bytes())
        .await
        .map_err(Into::<PackingError>::into)?;

    writer.write_all(&packed_message).await.map_err(Into::into)
}

async fn read_data_length<R>(reader: &mut R) -> Result<usize>
where
    R: AsyncReadExt + Unpin,
{
    let mut length_prefix = [0u8; MAX_MSG_LEN_BYTES];
    reader.read_exact(&mut length_prefix).await?;
    Ok(LenType::from_be_bytes(length_prefix) as usize)
}

pub async fn unpack_from_reader<Message, R>(reader: &mut R) -> Result<Message>
where
    Message: DeserializeOwned,
    R: AsyncReadExt + Unpin,
{
    let data_length = read_data_length(reader).await?;
    let mut data = vec![0u8; data_length];
    reader.read_exact(&mut data).await?;
    Ok(wire::deserialize(&data)?)
}
