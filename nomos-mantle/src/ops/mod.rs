pub mod blob;
mod inscribe;
pub mod opcode;
pub(crate) mod serde_;

use serde::{Deserialize, Serialize};

use crate::ops::{
    blob::BlobOp,
    inscribe::InscriptionOp,
    opcode::{BLOB, INSCRIBE},
};

pub type TxHash = [u8; 32];

#[derive(Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Op {
    Inscribe(
        #[serde(serialize_with = "serde_::serialize_op_variant::<{INSCRIBE}, InscribeOp, _>")]
        #[serde(deserialize_with = "serde_::deserialize_op_variant::<{INSCRIBE}, InscribeOp, _>")]
        InscriptionOp,
    ),
    Blob(
        #[serde(serialize_with = "serde_::serialize_op_variant::<{BLOB}, BlobOp, _>")]
        #[serde(deserialize_with = "serde_::deserialize_op_variant::<{BLOB}, BlobOp, _>")]
        BlobOp,
    ),
}

mod tests {
    use serde_json::{Value, json};

    use crate::ops::{Op, blob::BlobOp};

    #[test]
    fn test_serialize_deserialize_op() {
        let zeros = json!([
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0
        ]);
        let payload = json!({"channel": 0, "blob": zeros, "blob_size": 0, "after_tx": Value::Null, "signer": zeros});
        let repr = json!({"opcode": 0x01, "payload": payload});
        println!("{:?}", serde_json::to_string(&repr).unwrap());
        let op = Op::Blob(BlobOp {
            channel: 0,
            blob: [0; 32],
            blob_size: 0,
            after_tx: None,
            signer: [0; 32],
        });
        let serialized = serde_json::to_value(&op).unwrap();
        assert_eq!(serialized, repr);
        let deserialized = serde_json::from_value::<Op>(repr).unwrap();
        assert_eq!(deserialized, op);
    }
}
