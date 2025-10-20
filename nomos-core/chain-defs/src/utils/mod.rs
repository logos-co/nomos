pub mod ed25519_serde;
pub mod merkle;
pub mod select;

macro_rules! display_hex_bytes_newtype {
    ($newtype:ty) => {
        impl core::fmt::Display for $newtype {
            fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
                write!(f, "0x")?;
                for v in self.0 {
                    write!(f, "{:02x}", v)?;
                }
                Ok(())
            }
        }
    };
}

macro_rules! serde_bytes_newtype {
    ($newtype:ty, $len:expr) => {
        impl serde::Serialize for $newtype {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                if serializer.is_human_readable() {
                    const_hex::const_encode::<$len, false>(&self.0)
                        .as_str()
                        .serialize(serializer)
                } else {
                    self.0.serialize(serializer)
                }
            }
        }

        impl<'de> serde::Deserialize<'de> for $newtype {
            fn deserialize<D>(deserializer: D) -> Result<$newtype, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                if deserializer.is_human_readable() {
                    let s = <String>::deserialize(deserializer)?;
                    const_hex::decode_to_array(s)
                        .map(Self)
                        .map_err(serde::de::Error::custom)
                } else {
                    <[u8; $len]>::deserialize(deserializer).map(Self)
                }
            }
        }
    };
}

macro_rules! serde_big_bytes_newtype {
    ($newtype:ty, $len:expr) => {
        impl serde::Serialize for $newtype {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                if serializer.is_human_readable() {
                    const_hex::const_encode::<$len, false>(&self.0)
                        .as_str()
                        .serialize(serializer)
                } else {
                    self.0.serialize(serializer)
                }
            }
        }

        impl<'de> serde::Deserialize<'de> for $newtype {
            fn deserialize<D>(deserializer: D) -> Result<$newtype, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                if deserializer.is_human_readable() {
                    let s = <String>::deserialize(deserializer)?;
                    const_hex::decode_to_array(s)
                        .map(Self)
                        .map_err(serde::de::Error::custom)
                } else {
                    const CHUNK_SIZE: usize = 32;
                    const NUM_CHUNKS: usize = $len / CHUNK_SIZE;

                    // Compile-time assertion that $len is a multiple of 32
                    const _: [(); 0] = [(); $len % CHUNK_SIZE];

                    // Deserialize as [[u8; 32]; NUM_CHUNKS]
                    let chunks = <[[u8; CHUNK_SIZE]; NUM_CHUNKS]>::deserialize(deserializer)?;

                    let mut arr = [0u8; $len];
                    for (i, chunk) in chunks.iter().enumerate() {
                        let offset = i * CHUNK_SIZE;
                        arr[offset..offset + CHUNK_SIZE].copy_from_slice(chunk);
                    }

                    Ok(Self(arr))
                }
            }
        }
    };
}

pub(crate) use display_hex_bytes_newtype;
pub(crate) use serde_big_bytes_newtype;
pub(crate) use serde_bytes_newtype;
