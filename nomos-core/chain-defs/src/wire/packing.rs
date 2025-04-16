use std::io;

use futures::{AsyncReadExt, AsyncWriteExt};
use serde::{de::DeserializeOwned, Serialize};

use crate::wire;

type Result<T> = std::result::Result<T, io::Error>;

type LenType = u16;
const MAX_MSG_LEN_BYTES: usize = size_of::<LenType>();
const MAX_MSG_LEN: usize = LenType::MAX as usize;

struct MessageTooLargeError(usize);

impl From<MessageTooLargeError> for io::Error {
    fn from(value: MessageTooLargeError) -> Self {
        Self::new(
            io::ErrorKind::InvalidData,
            format!(
                "Message too large. Maximum size is {}. Actual size is {}",
                MAX_MSG_LEN, value.0
            ),
        )
    }
}

fn get_packed_message_size(packed_message: &[u8]) -> Result<usize> {
    let data_length = packed_message.len();
    if data_length > MAX_MSG_LEN {
        return Err(MessageTooLargeError(data_length).into());
    }
    Ok(data_length)
}

pub async fn pack_to_writer<Message, Writer>(message: &Message, writer: &mut Writer) -> Result<()>
where
    Message: Serialize + Sync,
    Writer: AsyncWriteExt + Send + Unpin,
{
    let packed_message = wire::serialize(message).map_err(io::Error::from)?;
    let data_length = get_packed_message_size(&packed_message)?;
    writer
        .write_all(&LenType::try_from(data_length).unwrap().to_be_bytes())
        .await?;

    writer.write_all(&packed_message).await
}
async fn read_data_length<R>(reader: &mut R) -> Result<usize>
where
    R: AsyncReadExt + Unpin,
{
    let mut length_prefix = [0u8; MAX_MSG_LEN_BYTES];
    reader.read_exact(&mut length_prefix).await?;
    let s = LenType::from_be_bytes(length_prefix) as usize;
    Ok(s)
}

pub fn unpack<M: DeserializeOwned>(data: &[u8]) -> Result<M> {
    wire::deserialize(data).map_err(io::Error::from)
}

pub async fn unpack_from_reader<Message, R>(reader: &mut R) -> Result<Message>
where
    Message: DeserializeOwned,
    R: AsyncReadExt + Unpin,
{
    let data_length = read_data_length(reader).await?;
    let mut data = vec![0u8; data_length];
    reader.read_exact(&mut data).await?;
    unpack(&data)
}

#[cfg(test)]
mod tests {
    use futures::io::BufReader;
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct TestStruct {
        id: u32,
        bytes: Vec<u8>,
        nested_struct: Option<Box<TestStruct>>,
    }

    #[test]
    fn pack_and_unpack() -> Result<()> {
        let message = TestStruct {
            id: 42,
            bytes: vec![1, 2, 3, 4],
            nested_struct: Some(Box::new(TestStruct {
                id: 43,
                bytes: vec![5, 6, 7, 8],
                nested_struct: None,
            })),
        };

        let packed_message = wire::serialize(&message).map_err(io::Error::from)?;
        let unpacked_message: TestStruct = unpack(&packed_message)?;

        assert_eq!(message, unpacked_message);
        Ok(())
    }

    #[tokio::test]
    async fn pack_to_writer_and_unpack_from_reader() -> Result<()> {
        let message = TestStruct {
            id: 42,
            bytes: vec![1, 2, 3, 4],
            nested_struct: Some(Box::new(TestStruct {
                id: 43,
                bytes: vec![5, 6, 7, 8],
                nested_struct: None,
            })),
        };

        let mut writer = Vec::new();
        pack_to_writer(&message, &mut writer).await?;

        let mut reader = BufReader::new(writer.as_slice());
        let unpacked_message: TestStruct = unpack_from_reader(&mut reader).await?;

        assert_eq!(message, unpacked_message);
        Ok(())
    }

    #[tokio::test]
    async fn pack_to_writer_too_large_message() {
        let message = TestStruct {
            id: 42,
            bytes: vec![1; MAX_MSG_LEN],
            nested_struct: None,
        };

        let mut writer = Vec::new();
        let res = pack_to_writer(&message, &mut writer).await;
        assert!(res.is_err());
        assert_eq!(res.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }
}
