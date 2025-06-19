use crate::mantle::gas::{Gas, GasConstants, GasPrice};

pub type SDPDeclareOp = nomos_sdp_core::DeclarationMessage;
pub type SDPWithdrawOp = nomos_sdp_core::WithdrawMessage;
// TODO: Abstract metadata
pub type SDPActiveOp = nomos_sdp_core::ActiveMessage<Vec<u8>>;

impl GasPrice for SDPDeclareOp {
    fn gas_price<Constants: GasConstants>(&self) -> Gas {
        Constants::DECLARE_BASE_GAS + Constants::DECLARE_LOCATOR_GAS * self.locators.len() as u64
    }
}

impl GasPrice for SDPWithdrawOp {
    fn gas_price<Constants: GasConstants>(&self) -> Gas {
        Constants::WITHDRAW_BASE_GAS
    }
}

impl GasPrice for SDPActiveOp {
    fn gas_price<Constants: GasConstants>(&self) -> Gas {
        Constants::ACTIVE_BASE_GAS
            + Constants::ACTIVE_BYTE_GAS * self.metadata.as_ref().map_or(0, |m| m.len() as u64)
    }
}
