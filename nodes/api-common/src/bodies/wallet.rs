pub mod balance {
    use axum::{
        http::StatusCode,
        response::{IntoResponse, Response},
    };
    use nomos_core::mantle::Value;
    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    pub struct WalletBalanceResponseBody {
        pub balance: Value,
    }

    impl IntoResponse for WalletBalanceResponseBody {
        fn into_response(self) -> Response {
            (StatusCode::OK, serde_json::to_string(&self).unwrap()).into_response()
        }
    }
}

pub mod transfer_funds {
    use axum::{
        http::StatusCode,
        response::{IntoResponse, Response},
    };
    use nomos_core::{
        header::HeaderId,
        mantle::{SignedMantleTx, Transaction as _, Value},
    };
    use serde::{Deserialize, Serialize};
    use zksign::PublicKey;

    #[derive(Serialize, Deserialize)]
    pub struct WalletTransferFundsRequestBody {
        pub tip: Option<HeaderId>,
        pub change_public_key: PublicKey,
        pub funding_public_keys: Vec<PublicKey>,
        pub recipient_public_key: PublicKey,
        pub amount: Value,
    }

    #[derive(Serialize, Deserialize)]
    pub struct WalletTransferFundsResponseBody {
        pub hash: nomos_core::mantle::tx::TxHash,
    }

    impl From<SignedMantleTx> for WalletTransferFundsResponseBody {
        fn from(value: SignedMantleTx) -> Self {
            Self {
                hash: value.mantle_tx.hash(),
            }
        }
    }

    impl IntoResponse for WalletTransferFundsResponseBody {
        fn into_response(self) -> Response {
            (StatusCode::CREATED, serde_json::to_string(&self).unwrap()).into_response()
        }
    }
}
