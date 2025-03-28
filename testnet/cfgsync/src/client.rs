use std::net::Ipv4Addr;

use reqwest::{Client, Response};
use serde::de::DeserializeOwned;

use crate::server::ClientIp;

async fn deserialize_response<Config: DeserializeOwned>(
    response: Response,
) -> Result<Config, String> {
    let body = response
        .text()
        .await
        .map_err(|error| format!("Failed to read response body: {error}"))?;
    let mut json_deserializer = serde_json::Deserializer::from_str(&body);
    serde_path_to_error::deserialize(&mut json_deserializer)
        .map_err(|error| format!("Failed to deserialize body: {error}"))
}

pub async fn get_config<Config: DeserializeOwned>(
    ip: Ipv4Addr,
    identifier: String,
    url: &str,
) -> Result<Config, String> {
    let client = Client::new();

    let response = client
        .post(url)
        .json(&ClientIp { ip, identifier })
        .send()
        .await
        .map_err(|err| format!("Failed to send IP announcement: {err}"))?;

    if !response.status().is_success() {
        return Err(format!("Server error: {:?}", response.status()));
    }

    deserialize_response(response).await
}
