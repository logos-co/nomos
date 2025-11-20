use ed25519_dalek::{Signer as _, SigningKey};
use nomos_core::mantle::{
    MantleTx, Op, OpProof, SignedMantleTx, Transaction as _,
    ledger::Tx as LedgerTx,
    ops::channel::{ChannelId, MsgId, inscribe::InscriptionOp},
};
use zksign::SecretKey;

#[must_use]
pub fn create_inscription_transaction_with_id(id: ChannelId) -> SignedMantleTx {
    let signing_key = SigningKey::from_bytes(&[0u8; 32]);
    let signer = signing_key.verifying_key();

    let inscription_op = InscriptionOp {
        channel_id: id,
        inscription: format!("Test channel inscription {id:?}").into_bytes(),
        parent: MsgId::root(),
        signer,
    };

    let mantle_tx = MantleTx {
        ops: vec![Op::ChannelInscribe(inscription_op)],
        ledger_tx: LedgerTx::new(vec![], vec![]),
        storage_gas_price: 0,
        execution_gas_price: 0,
    };

    let tx_hash = mantle_tx.hash();
    let signature = signing_key.sign(&tx_hash.as_signing_bytes());

    SignedMantleTx::new(
        mantle_tx,
        vec![OpProof::Ed25519Sig(signature)],
        SecretKey::multi_sign(&[], tx_hash.as_ref()).expect("zk signature generation"),
    )
    .expect("valid transaction")
}
