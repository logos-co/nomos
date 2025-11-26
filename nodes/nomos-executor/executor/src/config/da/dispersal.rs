use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    pub encoder_settings: EncoderConfig,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EncoderConfig {
    /// Whether to enable caching in the encoder
    pub with_cache: bool,
    /// Path to global KZG parameters file
    pub global_params_path: String,
}
