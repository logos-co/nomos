#[cfg(feature = "serde")]
pub mod serde {
    fn serialize_human_readable_bytes_array<const N: usize, S: serde::Serializer>(
        src: [u8; N],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        use serde::Serialize;
        const_hex::const_encode::<N, true>(&src)
            .as_str()
            .serialize(serializer)
    }

    pub fn serialize_bytes_array<const N: usize, S: serde::Serializer>(
        src: [u8; N],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            serialize_human_readable_bytes_array(src, serializer)
        } else {
            serializer.serialize_bytes(&src)
        }
    }

    fn deserialize_human_readable_bytes_array<'de, const N: usize, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<[u8; N], D::Error> {
        use serde::Deserialize;
        use std::borrow::Cow;
        let s: Cow<str> = Cow::deserialize(deserializer)?;
        let mut output = [0u8; N];
        const_hex::decode_to_slice(s.as_ref(), &mut output)
            .map(|_| output)
            .map_err(<D::Error as serde::de::Error>::custom)
    }

    fn deserialize_human_unreadable_bytes_array<
        'de,
        const N: usize,
        D: serde::Deserializer<'de>,
    >(
        deserializer: D,
    ) -> Result<[u8; N], D::Error> {
        use serde::Deserialize;
        <&[u8]>::deserialize(deserializer).and_then(|bytes| {
            if bytes.len() != N {
                Err(<D::Error as serde::de::Error>::invalid_length(
                    bytes.len(),
                    &format!("{N}").as_str(),
                ))
            } else {
                let mut output = [0u8; N];
                output.copy_from_slice(bytes);
                Ok(output)
            }
        })
    }

    pub fn deserialize_bytes_array<'de, const N: usize, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<[u8; N], D::Error> {
        if deserializer.is_human_readable() {
            deserialize_human_readable_bytes_array(deserializer)
        } else {
            deserialize_human_unreadable_bytes_array(deserializer)
        }
    }
}
