use super::CLIENT;
use reqwest::{Error, Response, Url};
use serde::Serialize;

pub async fn send_certificate<C>(node: &Url, cert: &C) -> Result<Response, Error>
where
    C: Serialize,
{
    const NODE_CERT_PATH: &str = "mempool-da/add";
    CLIENT
        .post(node.join(NODE_CERT_PATH).unwrap())
        .body(serde_json::to_string(cert).unwrap())
        .send()
        .await
}
