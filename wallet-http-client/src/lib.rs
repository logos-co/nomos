use common_http_client::{BasicAuthCredentials, CommonHttpClient, Error};
use nomos_core::{codec::SerializeOp as _, header::HeaderId};
use nomos_http_api_common::{
    bodies::{
        NoopBody,
        wallet::{
            balance::WalletBalanceResponseBody,
            transfer_funds::{WalletTransferFundsRequestBody, WalletTransferFundsResponseBody},
        },
    },
    paths::{WALLET_BALANCE, WALLET_TRANSFER},
};
use url::Url;
use zksign::PublicKey;

pub struct WalletHttpClient {
    client: CommonHttpClient,
}

impl WalletHttpClient {
    #[must_use]
    pub fn new(basic_auth: Option<BasicAuthCredentials>) -> Self {
        Self {
            client: CommonHttpClient::new(basic_auth),
        }
    }

    fn build_get_balance_url(
        base_url: &Url,
        wallet_address: PublicKey,
        tip: Option<HeaderId>,
    ) -> Result<Url, Error> {
        let Ok(wallet_address) = wallet_address.to_bytes() else {
            return Err(Error::Client(String::from(
                "The wallet address is not a valid public key.",
            )));
        };
        let wallet_address = hex::encode(wallet_address.iter().as_slice());
        let path = WALLET_BALANCE.replace(":public_key", wallet_address.as_str());
        let mut url = base_url.join(path.as_str()).map_err(Error::Url)?;
        if let Some(tip) = tip {
            let Ok(tip) = tip.to_bytes() else {
                return Err(Error::Client(String::from(
                    "The tip is not a valid header id.",
                )));
            };
            let tip = hex::encode(tip);
            let tip_query = format!("tip={tip}");
            url.set_query(Some(tip_query.as_str()));
        }
        Ok(url)
    }

    pub async fn get_balance(
        self,
        base_url: &Url,
        wallet_address: PublicKey,
        tip: Option<HeaderId>,
    ) -> Result<WalletBalanceResponseBody, Error> {
        let url = Self::build_get_balance_url(base_url, wallet_address, tip)?;
        self.client.get(url, Option::<&NoopBody>::None).await
    }

    pub async fn transfer_funds(
        self,
        base_url: Url,
        body: WalletTransferFundsRequestBody,
    ) -> Result<WalletTransferFundsResponseBody, Error> {
        let url = base_url.join(WALLET_TRANSFER).map_err(Error::Url)?;
        self.client.post(url, &body).await
    }
}
