use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum OpCode {
    Inscribe = 0x00,
    Blob = 0x01,
    SetChannelKeys = 0x02,
    Native = 0x03,
    SdpDeclare = 0x04,
    SdpWithdraw = 0x05,
    SdpActive = 0x06,
    LeaderClaim = 0x07,
}
