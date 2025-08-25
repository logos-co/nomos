use serde::Serialize;

use crate::{
    private_inputs::{PolPrivateInputs, PolPrivateInputsJson},
    public_inputs::{PolPublicInputs, PolPublicInputsJson},
};

#[derive(Clone, Serialize)]
#[serde(into = "PolInputsJson", rename_all = "snake_case")]
pub struct PolInputs {
    pub private: PolPrivateInputs,
    pub public: PolPublicInputs,
}

impl PolInputs {
    #[must_use]
    pub const fn from_public_and_private(
        public: PolPublicInputs,
        private: PolPrivateInputs,
    ) -> Self {
        Self { private, public }
    }
}

#[derive(Serialize)]
pub struct PolInputsJson {
    #[serde(flatten)]
    pub private: PolPrivateInputsJson,
    #[serde(flatten)]
    pub public: PolPublicInputsJson,
}

impl From<PolInputs> for PolInputsJson {
    fn from(inputs: PolInputs) -> Self {
        Self {
            private: (&inputs.private).into(),
            public: (&inputs.public).into(),
        }
    }
}
