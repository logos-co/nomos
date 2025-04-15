use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct DispersalRequest<Metadata> {
    pub data: Vec<u8>,
    pub metadata: Metadata,
}
