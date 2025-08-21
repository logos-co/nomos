use groth16::{Bn254, Fr, Groth16PublicInput};

pub struct PolPublicInputs {
    slot_number: Groth16PublicInput,
    epoch_nonce: Groth16PublicInput,
    lottery_0: Groth16PublicInput,
    lottery_1: Groth16PublicInput,
    merkle_root: Groth16PublicInput,
    latest_root: Groth16PublicInput,
    leader_pk: Groth16PublicInput,
    voucher_cml: Groth16PublicInput,
    entropy_contribution: Groth16PublicInput,
}

pub struct PolPublicInputsData {
    slot_number: usize,
    epoch_nonce: usize,
    total_stake: usize,
}

impl TryFrom<PolPublicInputsData> for PolPublicInputs {
    type Error = String;

    fn try_from(value: PolPublicInputsData) -> Result<Self, Self::Error> {
        todo!()
    }
}

impl PolPublicInputs {
    pub const fn to_inputs(&self) -> [Fr; 9] {
        [
            self.slot_number.into_inner(),
            self.epoch_nonce.into_inner(),
            self.lottery_0.into_inner(),
            self.lottery_1.into_inner(),
            self.merkle_root.into_inner(),
            self.latest_root.into_inner(),
            self.leader_pk.into_inner(),
            self.voucher_cml.into_inner(),
            self.entropy_contribution.into_inner(),
        ]
    }
}
