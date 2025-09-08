use groth16::{Field as _, Fr, Groth16Input, Groth16InputDeser};
use serde::{Deserialize, Serialize};

#[derive(Clone)]
pub struct PoQBlendInputs {
    core_sk: Groth16Input,
    core_path: Vec<Groth16Input>,
    core_path_selectors: Vec<Groth16Input>,
}

pub struct PoQBlendInputsData {
    pub core_sk: Fr,
    pub core_path: Vec<Fr>,
    pub core_path_selectors: Vec<bool>,
}

#[derive(Deserialize, Serialize)]
pub struct PoQBlendInputsJson {
    core_sk: Groth16InputDeser,
    core_path: Vec<Groth16InputDeser>,
    core_path_selectors: Vec<Groth16InputDeser>,
}

impl From<&PoQBlendInputs> for PoQBlendInputsJson {
    fn from(
        PoQBlendInputs {
            core_sk,
            core_path,
            core_path_selectors,
        }: &PoQBlendInputs,
    ) -> Self {
        Self {
            core_sk: core_sk.into(),
            core_path: core_path.iter().map(Into::into).collect(),
            core_path_selectors: core_path_selectors.iter().map(Into::into).collect(),
        }
    }
}

impl From<PoQBlendInputsData> for PoQBlendInputs {
    fn from(
        PoQBlendInputsData {
            core_sk,
            core_path,
            core_path_selectors,
        }: PoQBlendInputsData,
    ) -> Self {
        Self {
            core_sk: core_sk.into(),
            core_path: core_path.into_iter().map(Into::into).collect(),
            core_path_selectors: core_path_selectors
                .into_iter()
                .map(|value: bool| Groth16Input::new(if value { Fr::ONE } else { Fr::ZERO }))
                .collect(),
        }
    }
}
