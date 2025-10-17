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
    proofs::zksig::ZkSignaturePublic,
    sdp::{DeclarationId, Locator, ProviderId, ServiceType, ZkPublicKey as SdpZkPublicKey},
};

// ==============================================================================
// Top-Level Transaction Decoders
// ==============================================================================

pub fn decode_signed_mantle_tx(input: &[u8]) -> IResult<&[u8], SignedMantleTx> {
    // SignedMantleTx = MantleTx OpsProofs LedgerTxProof
    let (input, mantle_tx) = decode_mantle_tx(input)?;
    let (input, ops_proofs) = decode_ops_proofs(input, &mantle_tx.ops)?;
    let (input, ledger_tx_proof) = decode_zk_signature(input)?;

    // Use new_unverified since we're just decoding, verification happens separately
    #[cfg(any(test, debug_assertions))]
    {
        Ok((
            input,
            SignedMantleTx::new_unverified(
                mantle_tx,
                ops_proofs.into_iter().map(Some).collect(),
                ledger_tx_proof,
            ),
        ))
    }

    // In release mode without test/debug, we need to verify
    #[cfg(not(any(test, debug_assertions)))]
    {
        SignedMantleTx::new(mantle_tx, ops_proofs, ledger_tx_proof)
            .map(|tx| (input, tx))
            .map_err(|_| nom::Err::Error(Error::new(input, ErrorKind::Verify)))
    }
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
    let (input, metadata_vec) =
        map(take(metadata_len as usize), |b: &[u8]| b.to_vec()).parse(input)?;

    Ok((
        input,
        SDPActiveOp {
            declaration_id,
            nonce,
            metadata: if metadata_vec.is_empty() {
                None
            } else {
                Some(metadata_vec)
            },
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
        Op::ChannelInscribe(_) | Op::ChannelBlob(_) => {
            map(decode_ed25519_signature, OpProof::Ed25519Sig).parse(input)
        }

        // ZkAndEd25519SigsProof = ZkSignature Ed25519Signature
        Op::ChannelSetKeys(_) => {
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
        Op::SDPDeclare(_) | Op::SDPWithdraw(_) | Op::SDPActive(_) => {
            map(decode_zk_signature, OpProof::ZkSig).parse(input)
        }

        // ProofOfClaimProof = Groth16
        Op::LeaderClaim(_) => map(decode_groth16, |_proof| {
            panic!("OpProof::LeaderClaimProof not yet implemented");
        })
        .parse(input),
        Op::Native(_) => {
            unreachable!("Native ops should not be present in the proof")
        }
    }
}

// ==============================================================================
// Cryptographic Primitive Decoders
// ==============================================================================

fn decode_zk_signature(input: &[u8]) -> IResult<&[u8], DummyZkSignature> {
    // ZkSignature = Groth16
    map(decode_dummy_zk_signature, DummyZkSignature::from).parse(input)
}

fn decode_groth16(input: &[u8]) -> IResult<&[u8], CompressedGroth16Proof> {
    // Groth16 = 128BYTE
    map(take(128usize), |bytes: &[u8]| {
        let mut proof_bytes = [0u8; 128];
        proof_bytes.copy_from_slice(bytes);
        CompressedGroth16Proof::from_bytes(&proof_bytes)
    })
    .parse(input)
}

fn decode_dummy_zk_signature(input: &[u8]) -> IResult<&[u8], DummyZkSignature> {
    let (input, msg_hash) = decode_field_element(input)?;
    let (input, pks_len) = decode_u8(input)?;
    let (input, pks) = count(decode_field_element, pks_len as usize).parse(input)?;
    IResult::Ok((
        input,
        DummyZkSignature {
            public_inputs: ZkSignaturePublic { msg_hash, pks },
        },
    ))
}

fn decode_zk_public_key(input: &[u8]) -> IResult<&[u8], PublicKey> {
    // ZkPublicKey = FieldElement
    map(decode_field_element, PublicKey::new).parse(input)
}

fn decode_ed25519_public_key(input: &[u8]) -> IResult<&[u8], Ed25519PublicKey> {
    // Ed25519PublicKey = 32BYTE
    map_res(take(32usize), |bytes: &[u8]| {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(bytes);
        Ed25519PublicKey::from_bytes(&arr).map_err(|_| Error::new(bytes, ErrorKind::Fail))
    })
    .parse(input)
}

fn decode_ed25519_signature(input: &[u8]) -> IResult<&[u8], ed25519::Signature> {
    // Ed25519Signature = 64BYTE
    map(take(64usize), |bytes: &[u8]| {
        let mut arr = [0u8; 64];
        arr.copy_from_slice(bytes);
        ed25519::Signature::from_bytes(&arr)
    })
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
    map(take(32usize), |bytes: &[u8]| {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(bytes);
        arr
    })
    .parse(input)
}

// ==============================================================================
// Primitive Decoders
// ==============================================================================

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

    sig.as_bytes() // -> Fake implementation for dummy, will change to proof at some point
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
    // Metadata
    let metadata = op.metadata.as_ref().map_or(&[][..], |m| m.as_slice());
    bytes.extend(encode_uint32(metadata.len() as u32));
    bytes.extend(metadata);
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
        Op::Native(_) => {
            unimplemented!("Native operation is deprecated and will be removed");
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
fn encode_op_proof(proof: Option<&OpProof>, op: &Op) -> Vec<u8> {
    match (proof, op) {
        (Some(OpProof::Ed25519Sig(sig)), Op::ChannelInscribe(_) | Op::ChannelBlob(_)) => {
            encode_ed25519_signature(sig)
        }
        (
            Some(OpProof::ZkAndEd25519Sigs {
                zk_sig,
                ed25519_sig,
            }),
            Op::ChannelSetKeys(_),
        ) => {
            let mut bytes = encode_zk_signature(zk_sig);
            bytes.extend(encode_ed25519_signature(ed25519_sig));
            bytes
        }
        (Some(OpProof::ZkSig(sig)), Op::SDPDeclare(_) | Op::SDPWithdraw(_) | Op::SDPActive(_)) => {
            encode_zk_signature(sig)
        }
        (_, Op::LeaderClaim(_)) => {
            unimplemented!("ProofOfClaimProof not implemented");
        }
        _ => {
            panic!("Mismatch between proof type and operation type");
        }
    }
}

fn encode_ops_proofs(proofs: &[Option<OpProof>], ops: &[Op]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for (proof, op) in proofs.iter().zip(ops.iter()) {
        bytes.extend(encode_op_proof(proof.as_ref(), op));
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

#[cfg(test)]
mod tests {
    use ark_ff::Field as _;
    use ed25519::Signature;

    use super::*;

    fn dummy_zk_signature() -> DummyZkSignature {
        DummyZkSignature::prove(ZkSignaturePublic {
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
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // msg hash
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            1, // zk pks count
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // zk signature
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
                ledger_tx_proof: dummy_zk_signature(),
            }
        );
    }

    #[test]
    fn test_decode_signed_mantle_tx_with_inscribe() {
        #[rustfmt::skip]
        let data = [
            1,                         // OpCount=1
            0x00,                      // Opcode=ChannelInscribe
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, // ChannelId (32 bytes)
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
            5, 0, 0, 0,                // InscriptionLength =5u32
            b'h', b'e', b'l', b'l', b'o', // Inscription="hello"
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, // Parent (32 bytes)
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB,
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB,
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB,
            215, 90, 152, 1, 130, 177, 10, 183,             // Signer (Ed25519PublicKey) (32 bytes)
            213, 75, 254, 211, 201, 100, 7, 58,
            14, 225, 114, 243, 218, 166, 35, 37,
            175, 2, 26, 104, 247, 7, 81, 26,
            0, 0,                      // LedgerInputCount=0, LedgerOutputCount=0
            100, 0, 0, 0, 0, 0, 0, 0,  // ExecutionGasPrice=100u64
            50, 0, 0, 0, 0, 0, 0, 0,   // StorageGasPrice=50u64
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Ed25519Signature (64 bytes)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // dummy_zk_signature
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // msg hash
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            1, // zk pks count
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // zk signature
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
                    ops: vec![Op::ChannelInscribe(InscriptionOp {
                        channel_id: ChannelId::from([0xAA; 32]),
                        inscription: b"hello".to_vec(),
                        parent: MsgId::from([0xBB; 32]),
                        signer: Ed25519PublicKey::from_bytes(&[
                            215u8, 90, 152, 1, 130, 177, 10, 183, 213, 75, 254, 211, 201, 100, 7,
                            58, 14, 225, 114, 243, 218, 166, 35, 37, 175, 2, 26, 104, 247, 7, 81,
                            26
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
                ops_proofs: vec![Some(OpProof::Ed25519Sig(Signature::from_bytes(
                    &[0x00; 64]
                )))],
                ledger_tx_proof: dummy_zk_signature(),
            }
        );
    }

    #[test]
    fn test_decode_signed_mantle_tx_with_blob() {
        #[rustfmt::skip]
        let data = [
            1,                         // OpCount=1
            0x01,                      // Opcode=ChannelBlob
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, // ChannelId (32 bytes)
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, // BlobId (32 bytes)
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB,
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB,
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB,
            0, 4, 0, 0, 0, 0, 0, 0,    // BlobSize =1024u64
            10, 0, 0, 0, 0, 0, 0, 0,   // DaStorageGasPrice =10u64
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Parent (32 bytes)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            215, 90, 152, 1, 130, 177, 10, 183,             // Signer (Ed25519PublicKey) (32 bytes)
            213, 75, 254, 211, 201, 100, 7, 58,
            14, 225, 114, 243, 218, 166, 35, 37,
            175, 2, 26, 104, 247, 7, 81, 26,
            0, 0,                      // LedgerInputCount=0, LedgerOutputCount=0
            100, 0, 0, 0, 0, 0, 0, 0,  // ExecutionGasPrice=100u64
            50, 0, 0, 0, 0, 0, 0, 0,   // StorageGasPrice=50u64
            0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, // Ed25519Signature (64 bytes)
            0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD,
            0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD,
            0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD,
            0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD,
            0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD,
            0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD,
            0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD,
            // dummy_zk_signature
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // msg hash
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            1, // zk pks count
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // zk signature
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
                    ops: vec![Op::ChannelBlob(BlobOp {
                        channel: ChannelId::from([0xAA; 32]),
                        blob: [0xBB; 32],
                        blob_size: 1024,
                        da_storage_gas_price: 10,
                        parent: MsgId::from([0x00; 32]),
                        signer: Ed25519PublicKey::from_bytes(&[
                            215u8, 90, 152, 1, 130, 177, 10, 183, 213, 75, 254, 211, 201, 100, 7,
                            58, 14, 225, 114, 243, 218, 166, 35, 37, 175, 2, 26, 104, 247, 7, 81,
                            26
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
                ops_proofs: vec![Some(OpProof::Ed25519Sig(Signature::from_bytes(
                    &[0xDD; 64]
                )))],
                ledger_tx_proof: dummy_zk_signature(),
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
            2,                         // OpCount=2
            // Op 1: ChannelInscribe
            0x00,                      // Opcode=ChannelInscribe
            0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, // ChannelId (32 bytes)
            0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
            0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
            0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
            5, 0, 0, 0,                // InscriptionLength =5u32
            b'f', b'i', b'r', b's', b't', // Inscription="first"
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // Parent (32 bytes)
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            215, 90, 152, 1, 130, 177, 10, 183,             // Signer (Ed25519PublicKey) (32 bytes)
            213, 75, 254, 211, 201, 100, 7, 58,
            14, 225, 114, 243, 218, 166, 35, 37,
            175, 2, 26, 104, 247, 7, 81, 26,
            // Op 2: ChannelBlob
            0x01,                      // Opcode=ChannelBlob
            0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, // ChannelId (32 bytes)
            0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22,
            0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22,
            0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22, 0x22,
            0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, // BlobId (32 bytes)
            0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33,
            0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33,
            0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33, 0x33,
            0, 8, 0, 0, 0, 0, 0, 0,    // BlobSize =2048u64
            20, 0, 0, 0, 0, 0, 0, 0,   // DaStorageGasPrice =20u64
            0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, // Parent (32 bytes)
            0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44,
            0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44,
            0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44, 0x44,
            215, 90, 152, 1, 130, 177, 10, 183,             // Signer (Ed25519PublicKey) (32 bytes)
            213, 75, 254, 211, 201, 100, 7, 58,
            14, 225, 114, 243, 218, 166, 35, 37,
            175, 2, 26, 104, 247, 7, 81, 26,
            0, 0,                      // LedgerInputCount=0, LedgerOutputCount=0
            100, 0, 0, 0, 0, 0, 0, 0,  // ExecutionGasPrice=100u64
            50, 0, 0, 0, 0, 0, 0, 0,   // StorageGasPrice=50u64
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, // Ed25519Signature (64 bytes) for Op 1
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, // Ed25519Signature (64 bytes) for Op 2
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB,
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB,
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB,
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB,
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB,
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB,
            0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB,
            // dummy_zk_signature
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // msg hash
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            1, // zk pks count
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // zk signature
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
                    ops: vec![
                        Op::ChannelInscribe(InscriptionOp {
                            channel_id: ChannelId::from([0x11; 32]),
                            inscription: b"first".to_vec(),
                            parent: MsgId::from([0x00; 32]),
                            signer: Ed25519PublicKey::from_bytes(&[
                                215u8, 90, 152, 1, 130, 177, 10, 183, 213, 75, 254, 211, 201, 100,
                                7, 58, 14, 225, 114, 243, 218, 166, 35, 37, 175, 2, 26, 104, 247,
                                7, 81, 26
                            ])
                            .unwrap(),
                        }),
                        Op::ChannelBlob(BlobOp {
                            channel: ChannelId::from([0x22; 32]),
                            blob: [0x33; 32],
                            blob_size: 2048,
                            da_storage_gas_price: 20,
                            parent: MsgId::from([0x44; 32]),
                            signer: Ed25519PublicKey::from_bytes(&[
                                215u8, 90, 152, 1, 130, 177, 10, 183, 213, 75, 254, 211, 201, 100,
                                7, 58, 14, 225, 114, 243, 218, 166, 35, 37, 175, 2, 26, 104, 247,
                                7, 81, 26
                            ])
                            .unwrap(),
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
                    Some(OpProof::Ed25519Sig(Signature::from_bytes(&[0xAA; 64]))),
                    Some(OpProof::Ed25519Sig(Signature::from_bytes(&[0xBB; 64])))
                ],
                ledger_tx_proof: dummy_zk_signature(),
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
        assert_eq!(decoded_tx.ops.len(), original_tx.ops.len());
        assert_eq!(
            decoded_tx.ledger_tx.inputs.len(),
            original_tx.ledger_tx.inputs.len()
        );
        assert_eq!(
            decoded_tx.ledger_tx.outputs.len(),
            original_tx.ledger_tx.outputs.len()
        );
        assert_eq!(
            decoded_tx.execution_gas_price,
            original_tx.execution_gas_price
        );
        assert_eq!(decoded_tx.storage_gas_price, original_tx.storage_gas_price);
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
        assert_eq!(decoded_tx.ledger_tx.inputs.len(), 1);
        assert_eq!(decoded_tx.ledger_tx.outputs.len(), 1);
        assert_eq!(decoded_tx.ledger_tx.outputs[0].value, 1000);
        assert_eq!(decoded_tx.execution_gas_price, 100);
        assert_eq!(decoded_tx.storage_gas_price, 50);
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

        let original_tx = SignedMantleTx::new_unverified(mantle_tx, vec![], ledger_tx_proof);

        // Encode
        let encoded = encode_signed_mantle_tx(&original_tx);

        // Decode
        let (remaining, decoded_tx) = decode_signed_mantle_tx(&encoded).unwrap();

        // Verify
        assert!(remaining.is_empty());
        assert_eq!(decoded_tx.mantle_tx.ops.len(), 0);
        assert_eq!(decoded_tx.ops_proofs.len(), 0);
        assert_eq!(decoded_tx.mantle_tx.execution_gas_price, 100);
        assert_eq!(decoded_tx.mantle_tx.storage_gas_price, 50);
        // Verify the proof bytes match
        let original_proof_bytes = encode_zk_signature(&original_tx.ledger_tx_proof);
        let decoded_proof_bytes = encode_zk_signature(&decoded_tx.ledger_tx_proof);
        assert_eq!(original_proof_bytes, decoded_proof_bytes);
    }
}
