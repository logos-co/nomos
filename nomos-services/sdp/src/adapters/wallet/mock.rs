use std::convert::Infallible;

use ed25519_dalek::{Signer as _, SigningKey};
use nomos_core::{
    mantle::{Op, OpProof, SignedMantleTx, Transaction as _, tx_builder::MantleTxBuilder},
    proofs::zksig::{DummyZkSignature, ZkSignaturePublic},
    sdp::{ActiveMessage, DeclarationMessage, WithdrawMessage, ZkPublicKey},
};

use crate::adapters::wallet::SdpWalletAdapter;

pub struct MockWalletAdapter;

impl SdpWalletAdapter for MockWalletAdapter {
    type Error = Infallible;

    fn new() -> Self {
        Self {}
    }
    fn declare_tx(
        &self,
        tx_builder: MantleTxBuilder,
        declaration: Box<DeclarationMessage>,
    ) -> Result<SignedMantleTx, Self::Error> {
        // todo: this is for mock, we need signing key in production
        let signing_key = SigningKey::from_bytes(&[0u8; 32]);

        let declare_op = Op::SDPDeclare(*declaration.clone());
        let mantle_tx = tx_builder.push_op(declare_op).build();
        let tx_hash = mantle_tx.hash();

        let ed25519_sig = signing_key.sign(&tx_hash.as_signing_bytes());

        let zk_sig = DummyZkSignature::prove(&ZkSignaturePublic {
            msg_hash: tx_hash.into(),
            pks: vec![declaration.zk_id.0],
        });

        Ok(SignedMantleTx::new(
            mantle_tx,
            vec![OpProof::ZkAndEd25519Sigs {
                zk_sig,
                ed25519_sig,
            }],
            DummyZkSignature::prove(&ZkSignaturePublic {
                msg_hash: tx_hash.into(),
                pks: vec![],
            }),
        )
        .expect("Transaction with valid signature should be valid"))
    }

    fn withdraw_tx(
        &self,
        tx_builder: MantleTxBuilder,
        withdrawn_message: WithdrawMessage,
        zk_id: ZkPublicKey,
    ) -> Result<SignedMantleTx, Self::Error> {
        // Build the Op
        let withdraw_op = Op::SDPWithdraw(withdrawn_message);
        let mantle_tx = tx_builder.push_op(withdraw_op).build();
        let tx_hash = mantle_tx.hash();

        // From spec: ZkSignature_verify(txhash, signature, [locked_note.pk,
        // declare_info.zk_id])
        let zk_signature = DummyZkSignature::prove(&ZkSignaturePublic {
            msg_hash: tx_hash.into(),
            pks: vec![zk_id.0],
        });

        Ok(SignedMantleTx::new(
            mantle_tx,
            vec![OpProof::ZkSig(zk_signature)],
            DummyZkSignature::prove(&ZkSignaturePublic {
                msg_hash: tx_hash.into(),
                pks: vec![],
            }),
        )
        .expect("Transaction with valid signature should be valid"))
    }

    fn active_tx(
        &self,
        tx_builder: MantleTxBuilder,
        active_message: ActiveMessage,
        zk_id: ZkPublicKey,
    ) -> Result<SignedMantleTx, Self::Error> {
        let active_op = Op::SDPActive(active_message);
        let mantle_tx = tx_builder.push_op(active_op).build();
        let tx_hash = mantle_tx.hash();

        let zk_signature = DummyZkSignature::prove(&ZkSignaturePublic {
            msg_hash: tx_hash.into(),
            pks: vec![zk_id.0],
        });

        Ok(SignedMantleTx::new(
            mantle_tx,
            vec![OpProof::ZkSig(zk_signature)],
            DummyZkSignature::prove(&ZkSignaturePublic {
                msg_hash: tx_hash.into(),
                pks: vec![],
            }),
        )
        .expect("Transaction with valid signature should be valid"))
    }
}
