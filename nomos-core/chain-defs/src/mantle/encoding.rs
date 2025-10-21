use ed25519_dalek::VerifyingKey as Ed25519PublicKey;
use groth16::{CompressedGroth16Proof, Fr, fr_from_bytes};
use nom::{
    IResult, Parser as _,
    bytes::complete::take,
    combinator::{map, map_res},
    error::{Error, ErrorKind},
    multi::count,
    number::complete::{le_u32, le_u64, u8 as decode_u8},
};

use crate::{
    mantle::{
        MantleTx, Note, NoteId, SignedMantleTx, TxHash,
        keys::PublicKey,
        ledger::Tx as LedgerTx,
        ops::{
            Op, OpProof,
            channel::{
                ChannelId, MsgId, blob::BlobOp, inscribe::InscriptionOp, set_keys::SetKeysOp,
            },
            leader_claim::{LeaderClaimOp, RewardsRoot, VoucherNullifier},
            sdp::{SDPActiveOp, SDPDeclareOp, SDPWithdrawOp},
        },
    },
    sdp::{
        ActivityMetadata, DeclarationId, Locator, ProviderId, ServiceType,
        ZkPublicKey as SdpZkPublicKey,
    },
};

// ==============================================================================
// Top-Level Transaction Decoders
// ==============================================================================

pub fn decode_signed_mantle_tx(input: &[u8]) -> IResult<&[u8], SignedMantleTx> {
    // SignedMantleTx = MantleTx OpsProofs LedgerTxProof
    let (input, mantle_tx) = decode_mantle_tx(input)?;
    let (input, ops_proofs) = decode_ops_proofs(input, &mantle_tx.ops)?;
    let (input, ledger_tx_proof) = decode_zk_signature(input)?;

    let signed_tx = SignedMantleTx::new(mantle_tx, ops_proofs, ledger_tx_proof)
        .map_err(|_| nom::Err::Error(Error::new(input, ErrorKind::Verify)))?;

    Ok((input, signed_tx))
}

pub fn decode_mantle_tx(input: &[u8]) -> IResult<&[u8], MantleTx> {
    // MantleTx = Ops LedgerTx ExecutionGasPrice StorageGasPrice
    let (input, ops) = decode_ops(input)?;
    let (input, ledger_tx) = decode_ledger_tx(input)?;
    let (input, execution_gas_price) = decode_uint64(input)?;
    let (input, storage_gas_price) = decode_uint64(input)?;

    Ok((
        input,
        MantleTx {
            ops,
            ledger_tx,
            execution_gas_price,
            storage_gas_price,
        },
    ))
}

// ==============================================================================
// Operation List Decoders
// ==============================================================================

fn decode_ops(input: &[u8]) -> IResult<&[u8], Vec<Op>> {
    // Ops = OpCount *Op
    let (input, op_count) = decode_byte(input)?;
    count(decode_op, op_count as usize).parse(input)
}

fn decode_op(input: &[u8]) -> IResult<&[u8], Op> {
    // Op = Opcode OpPayload
    let (input, opcode) = decode_byte(input)?;

    match opcode {
        opcode::INSCRIBE => map(decode_channel_inscribe, Op::ChannelInscribe).parse(input),
        opcode::BLOB => map(decode_channel_blob, Op::ChannelBlob).parse(input),
        opcode::SET_CHANNEL_KEYS => map(decode_channel_set_keys, Op::ChannelSetKeys).parse(input),
        opcode::SDP_DECLARE => map(decode_sdp_declare, Op::SDPDeclare).parse(input),
        opcode::SDP_WITHDRAW => map(decode_sdp_withdraw, Op::SDPWithdraw).parse(input),
        opcode::SDP_ACTIVE => map(decode_sdp_active, Op::SDPActive).parse(input),
        opcode::LEADER_CLAIM => map(decode_leader_claim, Op::LeaderClaim).parse(input),
        _ => Err(nom::Err::Error(Error::new(input, ErrorKind::Fail))),
    }
}

// ==============================================================================
// Channel Operation Decoders
// ==============================================================================

fn decode_channel_inscribe(input: &[u8]) -> IResult<&[u8], InscriptionOp> {
    // ChannelInscribe = ChannelId Inscription Parent Signer
    // Inscription = UINT32 *BYTE
    // Signer = Ed25519PublicKey
    let (input, channel_id) = map(decode_hash32, ChannelId::from).parse(input)?;
    let (input, inscription_len) = decode_uint32(input)?;
    let (input, inscription) =
        map(take(inscription_len as usize), |b: &[u8]| b.to_vec()).parse(input)?;
    let (input, parent) = map(decode_hash32, MsgId::from).parse(input)?;
    let (input, signer) = decode_ed25519_public_key(input)?;

    Ok((
        input,
        InscriptionOp {
            channel_id,
            inscription,
            parent,
            signer,
        },
    ))
}

fn decode_channel_blob(input: &[u8]) -> IResult<&[u8], BlobOp> {
    // ChannelBlob = ChannelId BlobId BlobSize DaStorageGasPrice Parent Signer
    // Signer = Ed25519PublicKey
    let (input, channel) = map(decode_hash32, ChannelId::from).parse(input)?;
    let (input, blob) = decode_hash32(input)?;
    let (input, blob_size) = decode_uint64(input)?;
    let (input, da_storage_gas_price) = decode_uint64(input)?;
    let (input, parent) = map(decode_hash32, MsgId::from).parse(input)?;
    let (input, signer) = decode_ed25519_public_key.parse(input)?;

    Ok((
        input,
        BlobOp {
            channel,
            blob,
            blob_size,
            da_storage_gas_price,
            parent,
            signer,
        },
    ))
}

fn decode_channel_set_keys(input: &[u8]) -> IResult<&[u8], SetKeysOp> {
    // ChannelSetKeys = ChannelId KeyCount *Ed25519PublicKey
    let (input, channel) = map(decode_hash32, ChannelId::from).parse(input)?;
    let (input, key_count) = decode_byte(input)?;
    let (input, keys) = count(decode_ed25519_public_key, key_count as usize).parse(input)?;

    Ok((input, SetKeysOp { channel, keys }))
}

// ==============================================================================
// SDP Operation Decoders
// ==============================================================================

fn decode_sdp_declare(input: &[u8]) -> IResult<&[u8], SDPDeclareOp> {
    // SDPDeclare = ServiceType LocatorCount *Locator ProviderId ZkId LockedNoteId
    let (input, service_type_byte) = decode_byte(input)?;
    let service_type = match service_type_byte {
        0 => ServiceType::BlendNetwork,
        1 => ServiceType::DataAvailability,
        _ => return Err(nom::Err::Error(Error::new(input, ErrorKind::Fail))),
    };
    let (input, locator_count) = decode_byte(input)?;
    let (input, multiaddrs) = count(decode_locator, locator_count as usize).parse(input)?;
    let locators = multiaddrs.into_iter().map(Locator::new).collect();
    let (input, provider_key) = decode_ed25519_public_key(input)?;
    let provider_id = ProviderId(provider_key);
    let (input, zk_fr) = decode_field_element(input)?;
    let zk_id = SdpZkPublicKey(zk_fr);
    let (input, locked_note_id) = map(decode_field_element, NoteId).parse(input)?;

    Ok((
        input,
        SDPDeclareOp {
            service_type,
            locators,
            provider_id,
            zk_id,
            locked_note_id,
        },
    ))
}

const LOCATOR_BYTES_SIZE_LIMIT: usize = 329usize;

fn decode_locator(input: &[u8]) -> IResult<&[u8], multiaddr::Multiaddr> {
    // Locator = 2Byte *BYTE
    let (input, len_bytes) = take(2usize).parse(input)?;
    let len = u16::from_le_bytes([len_bytes[0], len_bytes[1]]) as usize;
    if len > LOCATOR_BYTES_SIZE_LIMIT {
        return Err(nom::Err::Error(Error::new(input, ErrorKind::LengthValue)));
    }
    map_res(take(len), |bytes: &[u8]| {
        multiaddr::Multiaddr::try_from(bytes.to_vec())
            .map_err(|_| Error::new(bytes, ErrorKind::Fail))
    })
    .parse(input)
}

fn decode_sdp_withdraw(input: &[u8]) -> IResult<&[u8], SDPWithdrawOp> {
    // SDPWithdraw = DeclarationId Nonce LockedNoteId
    let (input, declaration_id_bytes) = decode_hash32(input)?;
    let declaration_id = DeclarationId(declaration_id_bytes);
    let (input, nonce) = decode_uint64(input)?;
    let (input, locked_note_id) = map(decode_field_element, NoteId).parse(input)?;

    // NOTE: The ABNF specifies a LockedNoteId field, but the WithdrawMessage
    // struct does not have this field. We decode it but drop it for now.
    eprintln!(
        "WARNING: SDPWithdraw LockedNoteId field decoded but dropped. Declaration ID: {declaration_id:?}, nonce: {nonce}, locked_note: {locked_note_id:?}"
    );

    Ok((
        input,
        SDPWithdrawOp {
            declaration_id,
            nonce,
        },
    ))
}

fn decode_sdp_active(input: &[u8]) -> IResult<&[u8], SDPActiveOp> {
    // SDPActive = DeclarationId Nonce Metadata
    // Metadata = UINT32 *BYTE
    let (input, declaration_id_bytes) = decode_hash32(input)?;
    let declaration_id = DeclarationId(declaration_id_bytes);

    let (input, nonce) = decode_uint64(input)?;

    let (input, metadata_len) = decode_uint32(input)?;
    let (input, metadata_bytes) = take(metadata_len as usize).parse(input)?;

    let metadata = ActivityMetadata::from_metadata_bytes(metadata_bytes)
        .map_err(|_| nom::Err::Error(Error::new(input, ErrorKind::Fail)))?;

    Ok((
        input,
        SDPActiveOp {
            declaration_id,
            nonce,
            metadata,
        },
    ))
}

// ==============================================================================
// Leader Operation Decoders
// ==============================================================================

fn decode_leader_claim(input: &[u8]) -> IResult<&[u8], LeaderClaimOp> {
    // LeaderClaim = RewardsRoot VoucherNullifier
    let (input, rewards_root_fr) = decode_field_element(input)?;
    let (input, voucher_nullifier_fr) = decode_field_element(input)?;

    Ok((
        input,
        LeaderClaimOp {
            rewards_root: RewardsRoot::from(rewards_root_fr),
            voucher_nullifier: VoucherNullifier::from(voucher_nullifier_fr),
            // The mantle_tx_hash is not part of the wire format per ABNF spec
            // It should be filled in after decoding when the tx hash is computed
            mantle_tx_hash: TxHash::default(),
        },
    ))
}

// ==============================================================================
// Ledger Transaction Decoders
// ==============================================================================

fn decode_note(input: &[u8]) -> IResult<&[u8], Note> {
    // Note = Value ZkPublicKey
    let (input, value) = decode_uint64(input)?;
    let (input, pk) = decode_zk_public_key(input)?;

    Ok((input, Note::new(value, pk)))
}

fn decode_inputs(input: &[u8]) -> IResult<&[u8], Vec<NoteId>> {
    // Inputs = InputCount *NoteId
    let (input, input_count) = decode_byte(input)?;
    count(map(decode_field_element, NoteId), input_count as usize).parse(input)
}

fn decode_outputs(input: &[u8]) -> IResult<&[u8], Vec<Note>> {
    // Outputs = OutputCount *Note
    let (input, output_count) = decode_byte(input)?;
    count(decode_note, output_count as usize).parse(input)
}

fn decode_ledger_tx(input: &[u8]) -> IResult<&[u8], LedgerTx> {
    // LedgerTx = Inputs Outputs
    let (input, inputs) = decode_inputs(input)?;
    let (input, outputs) = decode_outputs(input)?;

    Ok((input, LedgerTx::new(inputs, outputs)))
}

// ==============================================================================
// Proof Decoders
// ==============================================================================

fn decode_ops_proofs<'a>(input: &'a [u8], ops: &[Op]) -> IResult<&'a [u8], Vec<OpProof>> {
    let mut remaining = input;
    let mut proofs = Vec::with_capacity(ops.len());

    for op in ops {
        let (new_remaining, proof) = decode_op_proof(remaining, op)?;
        proofs.push(proof);
        remaining = new_remaining;
    }

    Ok((remaining, proofs))
}

fn decode_op_proof<'a>(input: &'a [u8], op: &Op) -> IResult<&'a [u8], OpProof> {
    match op {
        // Ed25519SigProof = Ed25519Signature
        Op::ChannelInscribe(_) | Op::ChannelBlob(_) | Op::ChannelSetKeys(_) => {
            map(decode_ed25519_signature, OpProof::Ed25519Sig).parse(input)
        }

        // ZkAndEd25519SigsProof = ZkSignature Ed25519Signature
        Op::SDPDeclare(_) => {
            let (input, zk_sig) = decode_zk_signature(input)?;
            let (input, ed25519_sig) = decode_ed25519_signature(input)?;
            Ok((
                input,
                OpProof::ZkAndEd25519Sigs {
                    zk_sig,
                    ed25519_sig,
                },
            ))
        }

        // ZkSigProof = ZkSignature
        Op::SDPWithdraw(_) | Op::SDPActive(_) => {
            map(decode_zk_signature, OpProof::ZkSig).parse(input)
        }

        // ProofOfClaimProof = Groth16
        Op::LeaderClaim(_) => map(decode_groth16, |_proof| {
            panic!("OpProof::LeaderClaimProof not yet implemented");
        })
        .parse(input),
    }
}

// ==============================================================================
// Cryptographic Primitive Decoders
// ==============================================================================

fn decode_zk_signature(input: &[u8]) -> IResult<&[u8], DummyZkSignature> {
    // ZkSignature = Groth16
    // TODO: for now, signatures are dummy sigs
    map(decode_dummy_zk_signature, DummyZkSignature::from).parse(input)
}

const GROTH16_BYTES: usize = 128;
fn decode_groth16(input: &[u8]) -> IResult<&[u8], CompressedGroth16Proof> {
    // Groth16 = 128BYTE
    map(
        decode_array::<GROTH16_BYTES>,
        |proof: [u8; GROTH16_BYTES]| CompressedGroth16Proof::from_bytes(&proof),
    )
    .parse(input)
}

fn decode_dummy_zk_signature(input: &[u8]) -> IResult<&[u8], DummyZkSignature> {
    map(decode_array::<GROTH16_BYTES>, DummyZkSignature::from_bytes).parse(input)
}

fn decode_zk_public_key(input: &[u8]) -> IResult<&[u8], PublicKey> {
    // ZkPublicKey = FieldElement
    map(decode_field_element, PublicKey::new).parse(input)
}

const ED25519_PK_BYTES: usize = 32;
fn decode_ed25519_public_key(input: &[u8]) -> IResult<&[u8], Ed25519PublicKey> {
    // Ed25519PublicKey = 32BYTE
    map_res(
        decode_array::<ED25519_PK_BYTES>,
        |bytes: [u8; ED25519_PK_BYTES]| {
            Ed25519PublicKey::from_bytes(&bytes).map_err(|_| Error::new(bytes, ErrorKind::Fail))
        },
    )
    .parse(input)
}

const ED25519_SIG_BYTES: usize = 64;
fn decode_ed25519_signature(input: &[u8]) -> IResult<&[u8], ed25519::Signature> {
    // Ed25519Signature = 64BYTE
    map(
        decode_array::<ED25519_SIG_BYTES>,
        |bytes: [u8; ED25519_SIG_BYTES]| ed25519::Signature::from_bytes(&bytes),
    )
    .parse(input)
}

fn decode_field_element(input: &[u8]) -> IResult<&[u8], Fr> {
    // FieldElement = 32BYTE
    map_res(take(32usize), |bytes: &[u8]| {
        fr_from_bytes(bytes).map_err(|_| "Invalid field element")
    })
    .parse(input)
}

fn decode_hash32(input: &[u8]) -> IResult<&[u8], [u8; 32]> {
    // Hash32 = 32BYTE
    decode_array::<32>(input)
}

// ==============================================================================
// Primitive Decoders
// ==============================================================================
fn decode_array<const N: usize>(input: &[u8]) -> IResult<&[u8], [u8; N]> {
    map(take(N), |bytes: &[u8]| {
        let mut arr = [0u8; N];
        arr.copy_from_slice(bytes);
        arr
    })
    .parse(input)
}

fn decode_uint64(input: &[u8]) -> IResult<&[u8], u64> {
    // UINT64 = 8BYTE
    le_u64(input)
}

fn decode_uint32(input: &[u8]) -> IResult<&[u8], u32> {
    // UINT32 = 4BYTE
    le_u32(input)
}

fn decode_byte(input: &[u8]) -> IResult<&[u8], u8> {
    // Byte = OCTET
    decode_u8(input)
}

// ==============================================================================
// Binary Encoders
// ==============================================================================

use groth16::fr_to_bytes;

use super::ops::opcode;
use crate::proofs::zksig::DummyZkSignature;

/// Encode primitives
fn encode_uint64(value: u64) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn encode_uint32(value: u32) -> Vec<u8> {
    value.to_le_bytes().to_vec()
}

fn encode_byte(value: u8) -> Vec<u8> {
    vec![value]
}

fn encode_hash32(hash: &[u8; 32]) -> Vec<u8> {
    hash.to_vec()
}

fn encode_field_element(fr: &Fr) -> Vec<u8> {
    fr_to_bytes(fr).to_vec()
}

/// Encode cryptographic primitives
fn encode_ed25519_signature(sig: &ed25519::Signature) -> Vec<u8> {
    sig.to_bytes().to_vec()
}

fn encode_ed25519_public_key(key: &Ed25519PublicKey) -> Vec<u8> {
    key.to_bytes().to_vec()
}

fn encode_zk_signature(sig: &DummyZkSignature) -> Vec<u8> {
    // ZkSignature wraps ZkSignProof which is CompressedGroth16Proof
    // CompressedProof is 128 bytes: pi_a (32) + pi_b (64) + pi_c (32)
    // let proof = sig.as_proof();
    // let mut bytes = Vec::with_capacity(128);
    // bytes.extend_from_slice(proof.pi_a.as_slice());
    // bytes.extend_from_slice(proof.pi_b.as_slice());
    // bytes.extend_from_slice(proof.pi_c.as_slice());
    // bytes

    sig.as_bytes().to_vec() // -> Fake implementation for dummy, will change to proof at some point
}

/// Encode channel operations
fn encode_channel_inscribe(op: &InscriptionOp) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(encode_hash32(op.channel_id.as_ref()));
    bytes.extend(encode_uint32(op.inscription.len() as u32));
    bytes.extend(&op.inscription);
    bytes.extend(encode_hash32(op.parent.as_ref()));
    bytes.extend(encode_ed25519_public_key(&op.signer));
    bytes
}

fn encode_channel_blob(op: &BlobOp) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(encode_hash32(op.channel.as_ref()));
    bytes.extend(encode_hash32(&op.blob));
    bytes.extend(encode_uint64(op.blob_size));
    bytes.extend(encode_uint64(op.da_storage_gas_price));
    bytes.extend(encode_hash32(op.parent.as_ref()));
    bytes.extend(encode_ed25519_public_key(&op.signer));
    bytes
}

fn encode_channel_set_keys(op: &SetKeysOp) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(encode_hash32(op.channel.as_ref()));
    bytes.extend(encode_byte(op.keys.len() as u8));
    for key in &op.keys {
        bytes.extend(encode_ed25519_public_key(key));
    }
    bytes
}

/// Encode SDP operations
fn encode_locator(locator: &multiaddr::Multiaddr) -> Vec<u8> {
    let locator_bytes = locator.to_vec();
    assert!(locator_bytes.len() <= LOCATOR_BYTES_SIZE_LIMIT);
    let mut bytes = Vec::new();
    bytes.extend((locator_bytes.len() as u16).to_le_bytes());
    bytes.extend(locator_bytes);
    bytes
}

fn encode_sdp_declare(op: &SDPDeclareOp) -> Vec<u8> {
    let mut bytes = Vec::new();
    // ServiceType
    let service_type_byte = match op.service_type {
        ServiceType::BlendNetwork => 0u8,
        ServiceType::DataAvailability => 1u8,
    };
    bytes.extend(encode_byte(service_type_byte));
    // Locators
    bytes.extend(encode_byte(op.locators.len() as u8));
    for locator in &op.locators {
        bytes.extend(encode_locator(locator.as_ref()));
    }
    // ProviderId
    bytes.extend(encode_ed25519_public_key(&op.provider_id.0));
    // ZkId
    bytes.extend(encode_field_element(&op.zk_id.0));
    // LockedNoteId
    bytes.extend(encode_field_element(op.locked_note_id.as_ref()));
    bytes
}

fn encode_sdp_withdraw(op: &SDPWithdrawOp) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(encode_hash32(&op.declaration_id.0));
    bytes.extend(encode_uint64(op.nonce));
    // NOTE: ABNF specifies LockedNoteId field, but Rust struct doesn't have it
    // We encode zeros as a placeholder to match the wire format
    bytes.extend(encode_field_element(&Fr::from(0u64)));
    bytes
}

fn encode_sdp_active(op: &SDPActiveOp) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(encode_hash32(&op.declaration_id.0));
    bytes.extend(encode_uint64(op.nonce));

    // Metadata - convert ActivityMetadata to bytes
    let metadata_bytes = op
        .metadata
        .as_ref()
        .map(ActivityMetadata::to_metadata_bytes)
        .unwrap_or_default();

    bytes.extend(encode_uint32(metadata_bytes.len() as u32));
    bytes.extend(&metadata_bytes);
    bytes
}

/// Encode leader operations
fn encode_leader_claim(op: &LeaderClaimOp) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(encode_field_element(&op.rewards_root.into()));
    bytes.extend(encode_field_element(&op.voucher_nullifier.into()));
    bytes
}

/// Encode ledger transactions
fn encode_note(note: &Note) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(encode_uint64(note.value));
    bytes.extend(encode_field_element(note.pk.as_fr()));
    bytes
}

fn encode_inputs(inputs: &[NoteId]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(encode_byte(inputs.len() as u8));
    for input in inputs {
        bytes.extend(encode_field_element(input.as_ref()));
    }
    bytes
}

fn encode_outputs(outputs: &[Note]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(encode_byte(outputs.len() as u8));
    for output in outputs {
        bytes.extend(encode_note(output));
    }
    bytes
}

fn encode_ledger_tx(tx: &LedgerTx) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(encode_inputs(&tx.inputs));
    bytes.extend(encode_outputs(&tx.outputs));
    bytes
}

/// Encode operations
fn encode_op(op: &Op) -> Vec<u8> {
    let mut bytes = Vec::new();
    match op {
        Op::ChannelInscribe(op) => {
            bytes.extend(encode_byte(opcode::INSCRIBE));
            bytes.extend(encode_channel_inscribe(op));
        }
        Op::ChannelBlob(op) => {
            bytes.extend(encode_byte(opcode::BLOB));
            bytes.extend(encode_channel_blob(op));
        }
        Op::ChannelSetKeys(op) => {
            bytes.extend(encode_byte(opcode::SET_CHANNEL_KEYS));
            bytes.extend(encode_channel_set_keys(op));
        }
        Op::SDPDeclare(op) => {
            bytes.extend(encode_byte(opcode::SDP_DECLARE));
            bytes.extend(encode_sdp_declare(op));
        }
        Op::SDPWithdraw(op) => {
            bytes.extend(encode_byte(opcode::SDP_WITHDRAW));
            bytes.extend(encode_sdp_withdraw(op));
        }
        Op::SDPActive(op) => {
            bytes.extend(encode_byte(opcode::SDP_ACTIVE));
            bytes.extend(encode_sdp_active(op));
        }
        Op::LeaderClaim(op) => {
            bytes.extend(encode_byte(opcode::LEADER_CLAIM));
            bytes.extend(encode_leader_claim(op));
        }
    }
    bytes
}

fn encode_ops(ops: &[Op]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(encode_byte(ops.len() as u8));
    for op in ops {
        bytes.extend(encode_op(op));
    }
    bytes
}

/// Encode proofs
fn encode_op_proof(proof: &OpProof, op: &Op) -> Vec<u8> {
    match (proof, op) {
        (
            OpProof::Ed25519Sig(sig),
            Op::ChannelInscribe(_) | Op::ChannelBlob(_) | Op::ChannelSetKeys(_),
        ) => encode_ed25519_signature(sig),
        (
            OpProof::ZkAndEd25519Sigs {
                zk_sig,
                ed25519_sig,
            },
            Op::SDPDeclare(_),
        ) => {
            let mut bytes = encode_zk_signature(zk_sig);
            bytes.extend(encode_ed25519_signature(ed25519_sig));
            bytes
        }
        (OpProof::ZkSig(sig), Op::SDPWithdraw(_) | Op::SDPActive(_)) => encode_zk_signature(sig),
        (_, Op::LeaderClaim(_)) => {
            unimplemented!("ProofOfClaimProof not implemented");
        }
        _ => {
            panic!("Mismatch between proof type and operation type");
        }
    }
}

fn encode_ops_proofs(proofs: &[OpProof], ops: &[Op]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (proof, op) in proofs.iter().zip(ops.iter()) {
        bytes.extend(encode_op_proof(proof, op));
    }
    bytes
}

/// Encode top-level transactions
#[must_use]
pub fn encode_mantle_tx(tx: &MantleTx) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(encode_ops(&tx.ops));
    bytes.extend(encode_ledger_tx(&tx.ledger_tx));
    bytes.extend(encode_uint64(tx.execution_gas_price));
    bytes.extend(encode_uint64(tx.storage_gas_price));
    bytes
}

#[must_use]
pub fn encode_signed_mantle_tx(tx: &SignedMantleTx) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend(encode_mantle_tx(&tx.mantle_tx));
    bytes.extend(encode_ops_proofs(&tx.ops_proofs, &tx.mantle_tx.ops));
    bytes.extend(encode_zk_signature(&tx.ledger_tx_proof));
    bytes
}

pub(crate) fn predict_signed_mantle_tx_size(tx: &MantleTx) -> usize {
    let mantle_tx_size = encode_mantle_tx(tx).len();

    let ops_proofs_size = tx
        .ops
        .iter()
        .map(|op| match op {
            // Ed25519SigProof = Ed25519Signature
            Op::ChannelInscribe(_) | Op::ChannelBlob(_) | Op::ChannelSetKeys(_) => {
                ED25519_SIG_BYTES
            }

            // ZkAndEd25519SigsProof = ZkSignature Ed25519Signature
            Op::SDPDeclare(_) => GROTH16_BYTES + ED25519_SIG_BYTES,

            // ZkSigProof  = ZkSignature
            Op::SDPWithdraw(_) | Op::SDPActive(_) => GROTH16_BYTES,

            // ProofOfClaimProof = Groth16
            Op::LeaderClaim(_) => {
                unimplemented!("OpProof::LeaderClaimProof not yet implemented");
            }
        })
        .sum::<usize>();

    // LedgerTxProof = ZkSignature
    // ZkSignature   = Groth16
    let ledger_tx_proof_size = GROTH16_BYTES;

    mantle_tx_size + ops_proofs_size + ledger_tx_proof_size
}

#[cfg(test)]
mod tests {
    use ark_ff::Field as _;
    use ed25519::{Signature, signature::SignerMut as _};
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::{mantle::Transaction as _, proofs::zksig::ZkSignaturePublic};

    fn dummy_zk_signature() -> DummyZkSignature {
        DummyZkSignature::prove(&ZkSignaturePublic {
            msg_hash: Fr::ZERO,
            pks: [Fr::ZERO; 1].to_vec(),
        })
    }

    #[test]
    fn test_decode_primitives() {
        // Test UINT64
        let data = 42u64.to_le_bytes();
        let (remaining, value) = decode_uint64(&data).unwrap();
        assert_eq!(value, 42u64);
        assert!(remaining.is_empty());

        // Test UINT32
        let data = 123u32.to_le_bytes();
        let (remaining, value) = decode_uint32(&data).unwrap();
        assert_eq!(value, 123u32);
        assert!(remaining.is_empty());

        // Test Byte
        let data = [0xAB];
        let (remaining, value) = decode_byte(&data).unwrap();
        assert_eq!(value, 0xAB);
        assert!(remaining.is_empty());

        // Test Hash32
        let data = [0x42u8; 32];
        let (remaining, value) = decode_hash32(&data).unwrap();
        assert_eq!(value, [0x42u8; 32]);
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_decode_signed_mantle_tx_empty() {
        #[rustfmt::skip]
        let data = [
            0,                         // OpCount=0
            0, 0,                      // LedgerInputCount=0, LedgerOutputCount=0
            100, 0, 0, 0, 0, 0, 0, 0,  // ExecutionGasPrice=100u64
            50, 0, 0, 0, 0, 0, 0, 0,   // StorageGasPrice=50u64
            // dummy_zk_signature
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];

        let (remaining, signed_tx) = decode_signed_mantle_tx(&data).unwrap();

        assert!(remaining.is_empty());

        assert_eq!(
            signed_tx,
            SignedMantleTx {
                mantle_tx: MantleTx {
                    ops: vec![],
                    ledger_tx: LedgerTx {
                        inputs: vec![],
                        outputs: vec![],
                    },
                    execution_gas_price: 100,
                    storage_gas_price: 50,
                },
                ops_proofs: vec![],
                ledger_tx_proof: DummyZkSignature::from_bytes([0u8; 128])
            }
        );
    }

    #[test]
    fn test_decode_signed_mantle_tx_with_inscribe() {
        #[rustfmt::skip]
        let data = [
            1,                                              // OpCount
            0x00,                                           // OpCode
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, // ChannelId (32Byte)
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, //
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, //
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, //
            5, 0, 0, 0,                                     // InscriptionLength
            b'h', b'e', b'l', b'l', b'o',                   // Inscription
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, // Parent (32Byte)
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, //
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, //
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, //
            202, 147, 172,  23,   5,  24, 112, 113,         // Signer (32Byte)
            214, 123, 131, 199, 255,  14, 254, 129,         //
            8,   232, 236,  69,  48,  87,  93, 119,         //
            38,  135, 147,  51, 219, 218, 190, 124,         //
            0,                                              // LedgerInputCount
            0,                                              // LedgerOutputCount
            100, 0, 0, 0, 0, 0, 0, 0,                       // ExecutionGasPrice
             50, 0, 0, 0, 0, 0, 0, 0,                       // StorageGasPrice
            211, 102,  68,  60,  70, 179, 198, 132,         // Signature (64Byte)
            126, 141, 207, 182,  20, 128,   0,  42,         //
            233,  25,  70,   7,  81, 139, 245, 253,         //
              5,  75,  57, 249, 196, 162, 115, 129,         //
            167, 143,  89, 155, 103,  74, 204,  52,         //
            145, 167, 246,  16,  41, 121,  79, 142,         //
            188, 197, 171, 253, 229, 213, 196, 109,         //
             53,  13, 152,  10, 165,  52, 150,   7,         //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // ZkSignature (128Byte)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
        ];

        let (remaining, signed_tx) = decode_signed_mantle_tx(&data).unwrap();

        assert!(remaining.is_empty());

        assert_eq!(
            signed_tx,
            SignedMantleTx {
                mantle_tx: MantleTx {
                    ops: vec![Op::ChannelInscribe(InscriptionOp {
                        channel_id: ChannelId::from([0xAA; 32]),
                        inscription: b"hello".to_vec(),
                        parent: MsgId::from([0xBB; 32]),
                        signer: SigningKey::from_bytes(&[4u8; 32]).verifying_key()
                    })],
                    ledger_tx: LedgerTx {
                        inputs: vec![],
                        outputs: vec![],
                    },
                    execution_gas_price: 100,
                    storage_gas_price: 50
                },
                ops_proofs: vec![OpProof::Ed25519Sig(Signature::from_bytes(&[
                    211, 102, 68, 60, 70, 179, 198, 132, 126, 141, 207, 182, 20, 128, 0, 42, 233,
                    25, 70, 7, 81, 139, 245, 253, 5, 75, 57, 249, 196, 162, 115, 129, 167, 143, 89,
                    155, 103, 74, 204, 52, 145, 167, 246, 16, 41, 121, 79, 142, 188, 197, 171, 253,
                    229, 213, 196, 109, 53, 13, 152, 10, 165, 52, 150, 7,
                ]))],
                ledger_tx_proof: DummyZkSignature::from_bytes([0u8; 128])
            }
        );
    }

    #[test]
    fn test_decode_signed_mantle_tx_with_blob() {
        #[rustfmt::skip]
        let data = [
            1,                                              // OpCount
            0x01,                                           // Opcode
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, // ChannelId (32Bytes)
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, //
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, //
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, //
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, // BlobId (32Byte)
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, //
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, //
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, //
             0, 4, 0, 0, 0, 0, 0, 0,                        // BlobSize = 1024u64
            10, 0, 0, 0, 0, 0, 0, 0,                        // DaStorageGasPrice = 10u64
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Parent (32Byte)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            138, 136, 227, 221, 116,   9, 241, 149,         // Signer (32Byte)
            253,  82, 219,  45,  60, 186,  93, 114,         //
            202, 103,   9, 191,  29, 148,  18,  27,         //
            243, 116, 136,   1, 180,  15, 111,  92,         //
            0,                                              // LedgerInputCount
            0,                                              // LedgerOutputCount
            100, 0, 0, 0, 0, 0, 0, 0,                       // ExecutionGasPrice
             50, 0, 0, 0, 0, 0, 0, 0,                       // StorageGasPrice
            156, 246,   0,  26, 144, 216, 163, 126,         // Ed25519Signature (64Byte)
             70, 158, 248, 213, 158,   4,   2, 203,         //
            106, 168,  76,  93,  71,  24, 129,  10,         //
            143,  58, 178, 122,  42,  98, 107, 162,         //
            141, 143, 224,  56, 125, 109, 168, 226,         //
            168,   5,  57,  11,  14, 109,  80, 122,         //
            213, 107, 182,   3,  83, 140, 147, 154,         //
             36,  15, 167,  89,  41, 166, 142,   5,         //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // ZkSignature (128Byte)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
        ];

        let (remaining, signed_tx) = decode_signed_mantle_tx(&data).unwrap();

        assert!(remaining.is_empty());

        assert_eq!(
            signed_tx,
            SignedMantleTx {
                mantle_tx: MantleTx {
                    ops: vec![Op::ChannelBlob(BlobOp {
                        channel: ChannelId::from([0xAA; 32]),
                        blob: [0xBB; 32],
                        blob_size: 1024,
                        da_storage_gas_price: 10,
                        parent: MsgId::from([0x00; 32]),
                        signer: Ed25519PublicKey::from_bytes(&[
                            138, 136, 227, 221, 116, 9, 241, 149, 253, 82, 219, 45, 60, 186, 93,
                            114, 202, 103, 9, 191, 29, 148, 18, 27, 243, 116, 136, 1, 180, 15, 111,
                            92
                        ])
                        .unwrap(),
                    })],
                    ledger_tx: LedgerTx {
                        inputs: vec![],
                        outputs: vec![],
                    },
                    execution_gas_price: 100,
                    storage_gas_price: 50
                },
                ops_proofs: vec![OpProof::Ed25519Sig(Signature::from_bytes(&[
                    156, 246, 0, 26, 144, 216, 163, 126, 70, 158, 248, 213, 158, 4, 2, 203, 106,
                    168, 76, 93, 71, 24, 129, 10, 143, 58, 178, 122, 42, 98, 107, 162, 141, 143,
                    224, 56, 125, 109, 168, 226, 168, 5, 57, 11, 14, 109, 80, 122, 213, 107, 182,
                    3, 83, 140, 147, 154, 36, 15, 167, 89, 41, 166, 142, 5
                ]))],
                ledger_tx_proof: DummyZkSignature::from_bytes([0u8; 128])
            }
        );
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Data can be extracted, but it is just used locally"
    )]
    #[test]
    fn test_decode_signed_mantle_tx_with_multiple_ops() {
        #[rustfmt::skip]
        let data = [
            2,                                              // OpCount=2
                                                            // Op 1: ChannelInscribe
            0x00,                                           // Opcode=ChannelInscribe
            0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, // ChannelId (32Byte)
            0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, //
            0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, //
            0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, //
            5, 0, 0, 0,                                     // InscriptionLength =5u32
            b'f', b'i', b'r', b's', b't',                   // Inscription="first"
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Parent (32Byte)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            202, 147, 172,  23,   5,  24, 112, 113,         // Signer (32Byte)
            214, 123, 131, 199, 255,  14, 254, 129,         //
              8, 232, 236,  69,  48,  87,  93, 119,         //
             38, 135, 147,  51, 219, 218, 190, 124,         //
                                                            // Op 2: ChannelBlob
            0x01,                                           // Opcode=ChannelBlob
            0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, // ChannelId (32Byte)
            0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, //
            0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, //
            0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, //
            0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, // BlobId (32Byte)
            0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, //
            0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, //
            0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, //
            0, 8, 0, 0, 0, 0, 0, 0,                         // BlobSize =2048u64
            20, 0, 0, 0, 0, 0, 0, 0,                        // DaStorageGasPrice =20u64
            0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, // Parent (32Byte)
            0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, //
            0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, //
            0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, //
            202, 147, 172,  23,   5,  24, 112, 113,         // Signer (32Byte)
            214, 123, 131, 199, 255,  14, 254, 129,         //
              8, 232, 236,  69,  48,  87,  93, 119,         //
             38, 135, 147,  51, 219, 218, 190, 124,         //
            0,                                              // LedgerInputCount=0
            0,                                              // LedgerOutputCount=0
            100, 0, 0, 0, 0, 0, 0, 0,                       // ExecutionGasPrice=100u64
            50, 0, 0, 0, 0, 0, 0, 0,                        // StorageGasPrice=50u64
            194, 238, 118,  72, 128, 151,  84, 132,         // Ed25519Signature (64Byte)
            132, 227,  59,  66, 117, 238, 229, 167,         // -- for Op 1
            230, 241,  80, 118, 195,  38,  74,  16,         //
            210,  95,  80, 172,  91, 215, 237,  13,         //
            188, 108,  56,  40, 248,   8, 141,  92,         //
              9, 131,  24,  53, 220, 161, 222, 226,         //
            142, 168, 215, 187,  48,  79,  10,  71,         //
            134,  12, 127, 196, 163, 109, 219,  13,         //
            194, 238, 118,  72, 128, 151,  84, 132,         // Ed25519Signature (64Byte)
            132, 227,  59,  66, 117, 238, 229, 167,         // -- for Op 2
            230, 241,  80, 118, 195,  38,  74,  16,         //
            210,  95,  80, 172,  91, 215, 237,  13,         //
            188, 108,  56,  40, 248,   8, 141,  92,         //
              9, 131,  24,  53, 220, 161, 222, 226,         //
            142, 168, 215, 187,  48,  79,  10,  71,         //
            134,  12, 127, 196, 163, 109, 219,  13,         //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // ZkSignature (128Byte)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, //
        ];

        let (remaining, signed_tx) = decode_signed_mantle_tx(&data).unwrap();

        assert!(remaining.is_empty());

        let signer = Ed25519PublicKey::from_bytes(&[
            202, 147, 172, 23, 5, 24, 112, 113, 214, 123, 131, 199, 255, 14, 254, 129, 8, 232, 236,
            69, 48, 87, 93, 119, 38, 135, 147, 51, 219, 218, 190, 124,
        ])
        .unwrap();

        let signer_sig = Signature::from_bytes(&[
            194, 238, 118, 72, 128, 151, 84, 132, 132, 227, 59, 66, 117, 238, 229, 167, 230, 241,
            80, 118, 195, 38, 74, 16, 210, 95, 80, 172, 91, 215, 237, 13, 188, 108, 56, 40, 248, 8,
            141, 92, 9, 131, 24, 53, 220, 161, 222, 226, 142, 168, 215, 187, 48, 79, 10, 71, 134,
            12, 127, 196, 163, 109, 219, 13,
        ]);

        assert_eq!(
            signed_tx,
            SignedMantleTx {
                mantle_tx: MantleTx {
                    ops: vec![
                        Op::ChannelInscribe(InscriptionOp {
                            channel_id: ChannelId::from([0x11; 32]),
                            inscription: b"first".to_vec(),
                            parent: MsgId::from([0x00; 32]),
                            signer,
                        }),
                        Op::ChannelBlob(BlobOp {
                            channel: ChannelId::from([0x22; 32]),
                            blob: [0x33; 32],
                            blob_size: 2048,
                            da_storage_gas_price: 20,
                            parent: MsgId::from([0x44; 32]),
                            signer,
                        })
                    ],
                    ledger_tx: LedgerTx {
                        inputs: vec![],
                        outputs: vec![],
                    },
                    execution_gas_price: 100,
                    storage_gas_price: 50
                },
                ops_proofs: vec![
                    OpProof::Ed25519Sig(signer_sig),
                    OpProof::Ed25519Sig(signer_sig)
                ],
                ledger_tx_proof: DummyZkSignature::from_bytes([0u8; 128]),
            }
        );
    }

    #[test]
    fn test_encode_decode_roundtrip_empty_tx() {
        // Create an empty MantleTx
        let original_tx = MantleTx {
            ops: vec![],
            ledger_tx: LedgerTx::new(vec![], vec![]),
            execution_gas_price: 100,
            storage_gas_price: 50,
        };

        // Encode
        let encoded = encode_mantle_tx(&original_tx);

        // Decode
        let (remaining, decoded_tx) = decode_mantle_tx(&encoded).unwrap();

        // Verify
        assert!(remaining.is_empty());
        assert_eq!(original_tx, decoded_tx);
    }

    #[test]
    fn test_encode_decode_roundtrip_with_ledger_tx() {
        use num_bigint::BigUint;

        // Create a MantleTx with ledger inputs and outputs
        let pk = PublicKey::from(BigUint::from(42u64));
        let note = Note::new(1000, pk);
        let note_id = NoteId(BigUint::from(123u64).into());

        let original_tx = MantleTx {
            ops: vec![],
            ledger_tx: LedgerTx::new(vec![note_id], vec![note]),
            execution_gas_price: 100,
            storage_gas_price: 50,
        };

        // Encode
        let encoded = encode_mantle_tx(&original_tx);

        // Decode
        let (remaining, decoded_tx) = decode_mantle_tx(&encoded).unwrap();

        // Verify
        assert!(remaining.is_empty());
        assert_eq!(original_tx, decoded_tx);
    }

    #[test]
    fn test_encode_decode_roundtrip_signed_tx() {
        // Create a simple SignedMantleTx
        let mantle_tx = MantleTx {
            ops: vec![],
            ledger_tx: LedgerTx::new(vec![], vec![]),
            execution_gas_price: 100,
            storage_gas_price: 50,
        };

        let ledger_tx_proof = dummy_zk_signature();

        let original_tx = SignedMantleTx::new(mantle_tx, vec![], ledger_tx_proof).unwrap();

        // Encode
        let encoded = encode_signed_mantle_tx(&original_tx);

        // Decode
        let (remaining, decoded_tx) = decode_signed_mantle_tx(&encoded).unwrap();

        // Verify
        assert!(remaining.is_empty());
        assert_eq!(original_tx, decoded_tx);
    }

    #[test]
    fn test_predict_signed_mantle_tx_size_empty_tx() {
        // Create an empty MantleTx
        let mantle_tx = MantleTx {
            ops: vec![],
            ledger_tx: LedgerTx::new(vec![], vec![]),
            execution_gas_price: 100,
            storage_gas_price: 50,
        };

        // Predict size
        let predicted_size = predict_signed_mantle_tx_size(&mantle_tx);

        // Create a signed tx and encode it to get actual size
        let signed_tx = SignedMantleTx::new(mantle_tx, vec![], dummy_zk_signature()).unwrap();
        let encoded = encode_signed_mantle_tx(&signed_tx);
        let actual_size = encoded.len();

        assert_eq!(predicted_size, actual_size);
    }

    #[test]
    fn test_predict_signed_mantle_tx_size_with_inscribe() {
        use ed25519_dalek::SigningKey;

        let mut signing_key = SigningKey::from_bytes(&[1; 32]);
        let inscribe_op = InscriptionOp {
            channel_id: ChannelId::from([0xAA; 32]),
            inscription: b"hello world".to_vec(),
            parent: MsgId::from([0xBB; 32]),
            signer: signing_key.verifying_key(),
        };

        let mantle_tx = MantleTx {
            ops: vec![Op::ChannelInscribe(inscribe_op)],
            ledger_tx: LedgerTx::new(vec![], vec![]),
            execution_gas_price: 100,
            storage_gas_price: 50,
        };

        // Predict size
        let predicted_size = predict_signed_mantle_tx_size(&mantle_tx);

        // Create a signed tx and encode it to get actual size
        let op_sig = signing_key.sign(&mantle_tx.hash().as_signing_bytes());
        let signed_tx = SignedMantleTx::new(
            mantle_tx,
            vec![OpProof::Ed25519Sig(op_sig)],
            dummy_zk_signature(),
        )
        .unwrap();
        let encoded = encode_signed_mantle_tx(&signed_tx);
        let actual_size = encoded.len();

        assert_eq!(predicted_size, actual_size);
    }

    #[test]
    fn test_predict_signed_mantle_tx_size_with_blob() {
        use ed25519_dalek::SigningKey;

        let mut signing_key = SigningKey::from_bytes(&[1; 32]);
        let blob_op = BlobOp {
            channel: ChannelId::from([0xCC; 32]),
            blob: [0xDD; 32],
            blob_size: 1024,
            da_storage_gas_price: 10,
            parent: MsgId::from([0xEE; 32]),
            signer: signing_key.verifying_key(),
        };

        let mantle_tx = MantleTx {
            ops: vec![Op::ChannelBlob(blob_op)],
            ledger_tx: LedgerTx::new(vec![], vec![]),
            execution_gas_price: 100,
            storage_gas_price: 50,
        };

        // Predict size
        let predicted_size = predict_signed_mantle_tx_size(&mantle_tx);

        // Create a signed tx and encode it to get actual size
        let blob_sig = signing_key.sign(&mantle_tx.hash().as_signing_bytes());
        let signed_tx = SignedMantleTx::new(
            mantle_tx,
            vec![OpProof::Ed25519Sig(blob_sig)],
            dummy_zk_signature(),
        )
        .unwrap();
        let encoded = encode_signed_mantle_tx(&signed_tx);
        let actual_size = encoded.len();

        assert_eq!(predicted_size, actual_size);
    }

    #[test]
    fn test_predict_signed_mantle_tx_size_with_set_keys() {
        use ed25519_dalek::SigningKey;

        let signing_key1 = SigningKey::from_bytes(&[1; 32]);
        let signing_key2 = SigningKey::from_bytes(&[2; 32]);
        let signing_key3 = SigningKey::from_bytes(&[3; 32]);

        let set_keys_op = SetKeysOp {
            channel: ChannelId::from([0xFF; 32]),
            keys: vec![
                signing_key1.verifying_key(),
                signing_key2.verifying_key(),
                signing_key3.verifying_key(),
            ],
        };

        let mantle_tx = MantleTx {
            ops: vec![Op::ChannelSetKeys(set_keys_op)],
            ledger_tx: LedgerTx::new(vec![], vec![]),
            execution_gas_price: 100,
            storage_gas_price: 50,
        };

        // Predict size
        let predicted_size = predict_signed_mantle_tx_size(&mantle_tx);

        // Create a signed tx and encode it to get actual size
        let dummy_ed25519_sig = Signature::from_bytes(&[0; 64]);
        let signed_tx = SignedMantleTx::new(
            mantle_tx,
            vec![OpProof::Ed25519Sig(dummy_ed25519_sig)],
            dummy_zk_signature(),
        )
        .unwrap();
        let encoded = encode_signed_mantle_tx(&signed_tx);
        let actual_size = encoded.len();

        assert_eq!(predicted_size, actual_size);
    }

    #[test]
    fn test_predict_signed_mantle_tx_size_with_sdp_declare() {
        use ed25519_dalek::SigningKey;
        use num_bigint::BigUint;

        let signing_key = SigningKey::from_bytes(&[1; 32]);
        let locator1: multiaddr::Multiaddr = "/ip4/127.0.0.1/tcp/8080".parse().unwrap();
        let locator2: multiaddr::Multiaddr = "/ip6/::1/tcp/9090".parse().unwrap();

        let sdp_declare_op = SDPDeclareOp {
            service_type: ServiceType::BlendNetwork,
            locators: vec![Locator::new(locator1), Locator::new(locator2)],
            provider_id: ProviderId(signing_key.verifying_key()),
            zk_id: SdpZkPublicKey(BigUint::from(42u64).into()),
            locked_note_id: NoteId(BigUint::from(123u64).into()),
        };

        let mantle_tx = MantleTx {
            ops: vec![Op::SDPDeclare(sdp_declare_op)],
            ledger_tx: LedgerTx::new(vec![], vec![]),
            execution_gas_price: 100,
            storage_gas_price: 50,
        };

        // Predict size
        let predicted_size = predict_signed_mantle_tx_size(&mantle_tx);

        // Create a signed tx and encode it to get actual size
        let signed_tx = SignedMantleTx::new(
            mantle_tx,
            vec![OpProof::ZkAndEd25519Sigs {
                zk_sig: dummy_zk_signature(),
                ed25519_sig: Signature::from_bytes(&[0u8; 64]),
            }],
            dummy_zk_signature(),
        )
        .unwrap();
        let encoded = encode_signed_mantle_tx(&signed_tx);
        let actual_size = encoded.len();

        assert_eq!(predicted_size, actual_size);
    }

    #[test]
    fn test_predict_signed_mantle_tx_size_with_sdp_withdraw() {
        let sdp_withdraw_op = SDPWithdrawOp {
            declaration_id: DeclarationId([0x11; 32]),
            nonce: 42,
        };

        let mantle_tx = MantleTx {
            ops: vec![Op::SDPWithdraw(sdp_withdraw_op)],
            ledger_tx: LedgerTx::new(vec![], vec![]),
            execution_gas_price: 100,
            storage_gas_price: 50,
        };

        // Predict size
        let predicted_size = predict_signed_mantle_tx_size(&mantle_tx);

        // Create a signed tx and encode it to get actual size
        let signed_tx = SignedMantleTx::new(
            mantle_tx,
            vec![OpProof::ZkSig(dummy_zk_signature())],
            dummy_zk_signature(),
        )
        .unwrap();
        let encoded = encode_signed_mantle_tx(&signed_tx);
        let actual_size = encoded.len();

        assert_eq!(predicted_size, actual_size);
    }

    // #[test]
    // fn test_predict_signed_mantle_tx_size_with_sdp_active() {
    //     let sdp_active_op = SDPActiveOp {
    //         declaration_id: DeclarationId([0x22; 32]),
    //         nonce: 99,
    //         metadata: Some(vec![1, 2, 3, 4, 5]),
    //     };

    //     let mantle_tx = MantleTx {
    //         ops: vec![Op::SDPActive(sdp_active_op)],
    //         ledger_tx: LedgerTx::new(vec![], vec![]),
    //         execution_gas_price: 100,
    //         storage_gas_price: 50,
    //     };

    //     // Predict size
    //     let predicted_size = predict_signed_mantle_tx_size(&mantle_tx);

    //     // Create a signed tx and encode it to get actual size
    //     let signed_tx = SignedMantleTx::new(
    //         mantle_tx,
    //         vec![OpProof::ZkSig(dummy_zk_signature())],
    //         dummy_zk_signature(),
    //     )
    //     .unwrap();
    //     let encoded = encode_signed_mantle_tx(&signed_tx);
    //     let actual_size = encoded.len();

    //     assert_eq!(predicted_size, actual_size);
    // }

    #[test]
    fn test_predict_signed_mantle_tx_size_with_multiple_ops() {
        use ed25519_dalek::SigningKey;

        let mut signing_key = SigningKey::from_bytes(&[1; 32]);

        let inscribe_op = InscriptionOp {
            channel_id: ChannelId::from([0xAA; 32]),
            inscription: b"test".to_vec(),
            parent: MsgId::from([0xBB; 32]),
            signer: signing_key.verifying_key(),
        };

        let blob_op = BlobOp {
            channel: ChannelId::from([0xCC; 32]),
            blob: [0xDD; 32],
            blob_size: 2048,
            da_storage_gas_price: 20,
            parent: MsgId::from([0xEE; 32]),
            signer: signing_key.verifying_key(),
        };

        let sdp_active_op = SDPActiveOp {
            declaration_id: DeclarationId([0x33; 32]),
            nonce: 55,
            metadata: None,
        };

        let mantle_tx = MantleTx {
            ops: vec![
                Op::ChannelInscribe(inscribe_op),
                Op::ChannelBlob(blob_op),
                Op::SDPActive(sdp_active_op),
            ],
            ledger_tx: LedgerTx::new(vec![], vec![]),
            execution_gas_price: 100,
            storage_gas_price: 50,
        };

        // Predict size
        let predicted_size = predict_signed_mantle_tx_size(&mantle_tx);

        let op_sig = signing_key.sign(&mantle_tx.hash().as_signing_bytes());
        // Create a signed tx and encode it to get actual size
        let signed_tx = SignedMantleTx::new(
            mantle_tx,
            vec![
                OpProof::Ed25519Sig(op_sig),
                OpProof::Ed25519Sig(op_sig),
                OpProof::ZkSig(dummy_zk_signature()),
            ],
            dummy_zk_signature(),
        )
        .unwrap();
        let encoded = encode_signed_mantle_tx(&signed_tx);
        let actual_size = encoded.len();

        assert_eq!(predicted_size, actual_size);
    }

    #[test]
    fn test_predict_signed_mantle_tx_size_with_ledger_inputs_outputs() {
        use num_bigint::BigUint;

        use crate::mantle::keys::PublicKey;

        let pk1 = PublicKey::from(BigUint::from(100u64));
        let pk2 = PublicKey::from(BigUint::from(200u64));

        let note1 = Note::new(1000, pk1);
        let note2 = Note::new(2000, pk2);

        let note_id1 = NoteId(BigUint::from(111u64).into());
        let note_id2 = NoteId(BigUint::from(222u64).into());
        let note_id3 = NoteId(BigUint::from(333u64).into());

        let mantle_tx = MantleTx {
            ops: vec![],
            ledger_tx: LedgerTx::new(vec![note_id1, note_id2, note_id3], vec![note1, note2]),
            execution_gas_price: 100,
            storage_gas_price: 50,
        };

        // Predict size
        let predicted_size = predict_signed_mantle_tx_size(&mantle_tx);

        // Create a signed tx and encode it to get actual size
        let signed_tx = SignedMantleTx::new(mantle_tx, vec![], dummy_zk_signature()).unwrap();
        let encoded = encode_signed_mantle_tx(&signed_tx);
        let actual_size = encoded.len();

        assert_eq!(predicted_size, actual_size);
    }

    #[test]
    fn test_predict_signed_mantle_tx_size_complex_scenario() {
        use ed25519_dalek::SigningKey;
        use num_bigint::BigUint;

        use crate::mantle::keys::PublicKey;

        let mut signing_key1 = SigningKey::from_bytes(&[1; 32]);
        let signing_key2 = SigningKey::from_bytes(&[2; 32]);

        let inscribe_op = InscriptionOp {
            channel_id: ChannelId::from([0x11; 32]),
            inscription: b"complex test inscription with more data".to_vec(),
            parent: MsgId::from([0x22; 32]),
            signer: signing_key1.verifying_key(),
        };

        let set_keys_op = SetKeysOp {
            channel: ChannelId::from([0x33; 32]),
            keys: vec![signing_key1.verifying_key(), signing_key2.verifying_key()],
        };

        let locator: multiaddr::Multiaddr = "/dns4/example.com/tcp/443".parse().unwrap();
        let sdp_declare_op = SDPDeclareOp {
            service_type: ServiceType::DataAvailability,
            locators: vec![Locator::new(locator)],
            provider_id: ProviderId(signing_key1.verifying_key()),
            zk_id: SdpZkPublicKey(BigUint::from(999u64).into()),
            locked_note_id: NoteId(BigUint::from(888u64).into()),
        };

        let pk = PublicKey::from(BigUint::from(500u64));
        let note = Note::new(5000, pk);
        let note_id = NoteId(BigUint::from(777u64).into());

        let mantle_tx = MantleTx {
            ops: vec![
                Op::ChannelInscribe(inscribe_op),
                Op::ChannelSetKeys(set_keys_op),
                Op::SDPDeclare(sdp_declare_op),
            ],
            ledger_tx: LedgerTx::new(vec![note_id], vec![note]),
            execution_gas_price: 150,
            storage_gas_price: 75,
        };

        // Predict size
        let predicted_size = predict_signed_mantle_tx_size(&mantle_tx);

        // Create a signed tx and encode it to get actual size
        let op_ed25519_sig = signing_key1.sign(&mantle_tx.hash().as_signing_bytes());
        let signed_tx = SignedMantleTx::new(
            mantle_tx,
            vec![
                OpProof::Ed25519Sig(op_ed25519_sig),
                OpProof::Ed25519Sig(op_ed25519_sig),
                OpProof::ZkAndEd25519Sigs {
                    zk_sig: dummy_zk_signature(),
                    ed25519_sig: op_ed25519_sig,
                },
            ],
            dummy_zk_signature(),
        )
        .unwrap();
        let encoded = encode_signed_mantle_tx(&signed_tx);
        let actual_size = encoded.len();

        assert_eq!(predicted_size, actual_size);
    }
}
