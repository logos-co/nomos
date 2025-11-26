use nom::{IResult, Parser as _, bytes::complete::take, number::complete::u8 as nom_u8};
use serde::{Deserialize, Serialize};

use crate::{
    blend::{PROOF_OF_QUOTA_SIZE, PROOF_OF_SELECTION_SIZE},
    sdp::{ACTIVE_METADATA_BLEND_TYPE, SessionNumber, parse_session_number},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ActivityProof {
    pub session: SessionNumber,
    #[serde(with = "serde_big_array::BigArray")]
    pub proof_of_quota: [u8; PROOF_OF_QUOTA_SIZE],
    pub proof_of_selection: [u8; PROOF_OF_SELECTION_SIZE],
}

const BLEND_ACTIVE_METADATA_VERSION_BYTE: u8 = 0x01;

impl ActivityProof {
    #[must_use]
    pub fn to_metadata_bytes(&self) -> Vec<u8> {
        let total_size = 2 // type + version byte
            + size_of::<SessionNumber>()
            + self.proof_of_quota.len()
            + self.proof_of_selection.len();

        let mut bytes = Vec::with_capacity(total_size);
        bytes.push(ACTIVE_METADATA_BLEND_TYPE);
        bytes.push(BLEND_ACTIVE_METADATA_VERSION_BYTE);
        bytes.extend(&self.session.to_le_bytes());
        bytes.extend(&self.proof_of_quota);
        bytes.extend(&self.proof_of_selection);
        bytes
    }

    /// Parse metadata bytes using nom combinators
    pub fn from_metadata_bytes(bytes: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(parse_activity_proof(bytes)
            .map_err(|e| format!("Failed to parse metadata: {e}"))?
            .1)
    }
}

fn parse_activity_proof(input: &[u8]) -> IResult<&[u8], ActivityProof> {
    let (input, metadata_type) = nom_u8(input)?;
    if metadata_type != ACTIVE_METADATA_BLEND_TYPE {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        )));
    }

    let (input, version) = nom_u8(input)?;
    if version != BLEND_ACTIVE_METADATA_VERSION_BYTE {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        )));
    }
    let (input, session) = parse_session_number(input)?;
    let (input, proof_of_quota) = parse_const_size_bytes::<PROOF_OF_QUOTA_SIZE>(input)?;
    let (input, proof_of_selection) = parse_const_size_bytes::<PROOF_OF_SELECTION_SIZE>(input)?;

    if !input.is_empty() {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Eof,
        )));
    }

    Ok((
        input,
        ActivityProof {
            session,
            proof_of_quota,
            proof_of_selection,
        },
    ))
}

fn parse_const_size_bytes<const N: usize>(input: &[u8]) -> IResult<&[u8], [u8; N]> {
    let (input, data) = take(N).parse(input)?;
    let data: [u8; N] = data
        .try_into()
        .map_err(|_| nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Fail)))?;
    Ok((input, data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sdp::ActivityMetadata;

    #[test]
    fn activity_proof_roundtrip() {
        let proof = ActivityProof {
            session: 10,
            proof_of_quota: [0u8; PROOF_OF_QUOTA_SIZE],
            proof_of_selection: [1u8; PROOF_OF_SELECTION_SIZE],
        };

        let bytes = proof.to_metadata_bytes();
        let decoded = ActivityProof::from_metadata_bytes(&bytes).unwrap();

        assert_eq!(proof, decoded);
    }

    #[test]
    fn activity_proof_invalid_version() {
        let proof = ActivityProof {
            session: 10,
            proof_of_quota: [0u8; PROOF_OF_QUOTA_SIZE],
            proof_of_selection: [1u8; PROOF_OF_SELECTION_SIZE],
        };
        let mut bytes = proof.to_metadata_bytes();
        bytes[0] = 0x99; // Invalid version

        let result = ActivityProof::from_metadata_bytes(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse"));
    }

    #[test]
    fn activity_proof_too_short() {
        let bytes = vec![BLEND_ACTIVE_METADATA_VERSION_BYTE, 0x01, 0x02]; // Only 3 bytes

        let result = ActivityProof::from_metadata_bytes(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Eof"));
    }

    #[test]
    fn activity_proof_too_long() {
        let proof = ActivityProof {
            session: 10,
            proof_of_quota: [0u8; PROOF_OF_QUOTA_SIZE],
            proof_of_selection: [1u8; PROOF_OF_SELECTION_SIZE],
        };
        let mut bytes = proof.to_metadata_bytes();
        bytes.push(0xFF); // An extra byte

        let result = ActivityProof::from_metadata_bytes(&bytes);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Eof"));
    }

    #[test]
    fn activity_metadata_roundtrip() {
        let proof = ActivityProof {
            session: 10,
            proof_of_quota: [0u8; PROOF_OF_QUOTA_SIZE],
            proof_of_selection: [1u8; PROOF_OF_SELECTION_SIZE],
        };
        let metadata = ActivityMetadata::Blend(proof.clone());

        let bytes = metadata.to_metadata_bytes();
        let decoded = ActivityMetadata::from_metadata_bytes(&bytes).unwrap();

        assert_eq!(metadata, decoded);

        let ActivityMetadata::Blend(decoded_proof) = decoded else {
            panic!("Unexpected ActivityMetadata variant");
        };
        assert_eq!(proof, decoded_proof);
    }
}
