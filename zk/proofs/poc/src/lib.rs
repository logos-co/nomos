//! # zk-poc
//!
//! ## Usage
//!
//! The library provides a single function, `prove`, which takes a set of
//! inputs and generates a proof. The function returns a tuple containing the
//! generated proof and the corresponding public inputs.
//! A normal flow of usage will involve the following steps:
//! 1. Fill out some `PoCChainData` with the public inputs
//! 2. Fill out some `PoCWalletData` with the private inputs
//! 3. Construct the `PoCWitnessInputs` from the `PoCChainData` and
//!    `PoCWalletData`
//! 4. Call `prove` with the `PoCWitnessInputs`
//! 5. Use the returned proof and public inputs to verify the proof
//!
//! ## Example
//!
//! ```ignore
//! use zk_poc::{prove, PoCChainInputs, PoCChainInputsData, PoCWalletInputs, PoCWalletInputsData};
//!
//! fn main() {
//!     let chain_data = PoCChainInputsData {..};
//!     let wallet_data = PoCWalletInputsData {..};
//!     let witness_inputs = PoCWitnessInputs::from_chain_and_wallet_data(chain_data, wallet_data);
//!     let (proof, inputs) = prove(&witness_inputs).unwrap();
//!     assert!(verify(&proof, &inputs).unwrap());
//! }

mod chain_inputs;
mod inputs;
mod proving_key;
mod verification_key;
mod wallet_inputs;
mod witness;

use core::fmt::Debug;
use std::error::Error;

pub use chain_inputs::{PoCChainInputs, PoCChainInputsData};
use groth16::{
    CompressedGroth16Proof, Groth16Input, Groth16InputDeser, Groth16Proof, Groth16ProofJsonDeser,
};
pub use inputs::PoCWitnessInputs;
use thiserror::Error;
pub use wallet_inputs::{PoCWalletInputs, PoCWalletInputsData};
pub use witness::Witness;

use crate::{
    inputs::{PoCVerifierInput, PoCVerifierInputJson},
    proving_key::POC_PROVING_KEY_PATH,
};

pub type PoCProof = CompressedGroth16Proof;

#[derive(Debug, Error)]
pub enum ProveError {
    #[error(transparent)]
    Io(std::io::Error),
    #[error(transparent)]
    Json(serde_json::Error),
    #[error("Error parsing Groth16 input: {0:?}")]
    Groth16JsonInput(<Groth16Input as TryFrom<Groth16InputDeser>>::Error),
    #[error(transparent)]
    Groth16JsonProof(<Groth16Proof as TryFrom<Groth16ProofJsonDeser>>::Error),
}

///
/// This function generates a proof for the given set of inputs.
///
/// # Arguments
/// - `inputs`: A reference to `PoCWitnessInputs`, which contains the necessary
///   data to generate the witness and construct the proof.
///
/// # Returns
/// - `Ok((PoCProof, PoCVerifierInput))`: On success, returns a tuple containing
///   the generated proof (`PoCProof`) and the corresponding public inputs
///   (`PoCVerifierInput`).
/// - `Err(ProveError)`: On failure, returns an error of type `ProveError`,
///   which can occur due to I/O errors or JSON (de)serialization errors.
///
/// # Errors
/// - Returns a `ProveError::Io` if an I/O error occurs while generating the
///   witness or proving from contents.
/// - Returns a `ProveError::Json` if there is an error during JSON
///   serialization or deserialization.
pub fn prove(inputs: &PoCWitnessInputs) -> Result<(PoCProof, PoCVerifierInput), ProveError> {
    let witness = witness::generate_witness(inputs).map_err(ProveError::Io)?;
    let (proof, verifier_inputs) =
        circuits_prover::prover_from_contents(POC_PROVING_KEY_PATH.as_path(), witness.as_ref())
            .map_err(ProveError::Io)?;
    let proof: Groth16ProofJsonDeser = serde_json::from_slice(&proof).map_err(ProveError::Json)?;
    let verifier_inputs: PoCVerifierInputJson =
        serde_json::from_slice(&verifier_inputs).map_err(ProveError::Json)?;
    let proof: Groth16Proof = proof.try_into().map_err(ProveError::Groth16JsonProof)?;
    Ok((
        CompressedGroth16Proof::try_from(&proof).unwrap(),
        verifier_inputs
            .try_into()
            .map_err(ProveError::Groth16JsonInput)?,
    ))
}

#[derive(Debug)]
pub enum VerifyError {
    Expansion,
    ProofVerify(Box<dyn Error>),
}

///
/// This function verifies a proof against a set of public inputs.
///
/// # Arguments
///
/// - `proof`: A reference to the proof (`PoCProof`) that needs verification.
/// - `public_inputs`: A reference to `PoCVerifierInput`, which contains the
///   public inputs against which the proof is verified.
///
/// # Returns
///
/// - `Ok(true)`: If the proof is successfully verified against the public
///   inputs.
/// - `Ok(false)`: If the proof is invalid when compared with the public inputs.
/// - `Err`: If an error occurs during the verification process.
///
/// # Errors
///
/// - Returns an error if there is an issue with the verification key or the
///   underlying verification process fails.
pub fn verify(proof: &PoCProof, public_inputs: &PoCVerifierInput) -> Result<bool, VerifyError> {
    let inputs = public_inputs.to_inputs();
    let expanded_proof = Groth16Proof::try_from(proof).map_err(|_| VerifyError::Expansion)?;
    groth16::groth16_verify(verification_key::POC_VK.as_ref(), &expanded_proof, &inputs)
        .map_err(|e| VerifyError::ProofVerify(Box::new(e)))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use num_bigint::BigUint;

    use super::*;

    #[test]
    fn test_full_flow() {
        let chain_data = PoCChainInputsData {
            voucher_root: BigUint::from_str(
                "4168133624007432145884017655811318012261843844909572808010590900685509230600",
            )
            .unwrap()
            .into(),
            mantle_tx_hash: BigUint::from_str(
                "19243327178185008752516381292121932993324674792423275941647764942218727361600",
            )
            .unwrap()
            .into(),
        };
        let wallet_data = PoCWalletInputsData {
            secret_voucher: BigUint::from_str(
                "12648995811387757430670010200107400342740118854373982042767012129445802142461",
            )
            .unwrap()
            .into(),
            voucher_merkle_path: [
                "17419382439654953040164193480497791244949600305510513527925376855091880574919",
                "11467654816169861897143103331018439264614156920502955220502362583206087456274",
                "19653552388383226620248262762484769376201724982114742850897129490857548701642",
                "4333317464554997673381632415481196460896133925406274072380534750639894145403",
                "3943111857790222389368539535462184018302630626377588950225550152989338971167",
                "4652869800901160125832853725446738586093178193436502461745766097659344660700",
                "20117738612546318388802765567827231667448918316071856195843397294730579196230",
                "3531403744553523232215782496458347952295152891962461992865141108371448977700",
                "12390927977404943133538238076746370049305309081888998217486636155217839494572",
                "14849197005169902756252197861393605672024151187011486949833891499961200843709",
                "18660128566620576545010823842232038051647558013588783235917759907241234913424",
                "8260250137373301518186252772408574634460750973505719164280260670640212113279",
                "21041478124119451927475383032353943380483633274065972658376163446758840659375",
                "12540057593171088481199742353041896131772022823165711746100525834182018162620",
                "15418888839146622209824521892033919127998892220077463206601376152232800308128",
                "16723485614197215912442135420392944983759216535533724492622186320397123421787",
                "18815228895925211247967347591242335332690972617083342260447408624133009391789",
                "5427349793207910677518791188159903924819448511580961597101674069076995306778",
                "6097697927352770975623724397772020058802650464688248035281144559258295148775",
                "8196732625968579841406141793080807089696159623122758544698364569597368729051",
                "3442467447171045733054834904568124934159740554928586160489591596228240497952",
                "18843519886383555520339809002676326864887859486131161544846851857293285554411",
                "11736771741780272197114898386126223734278174394149680115884465450153381310355",
                "9626408241367840491848349352200974759736439966680779227978616973958001771444",
                "21195878875659961628634281702654073389312466194121209508828473264221402787470",
                "5560582947954072264164000867147352110442266707896829462160399386698092257273",
                "15891599785743618272879065421888700413781762399212843674581298615794715664714",
                "21149911073189397046557448254523845627956571987545420989163798342993801641542",
                "4797379228789536209761233142838726490050648744057542677523962592635276471654",
                "9393239182856141461953440501759262129794492561069848425805485149731209279199",
                "19780158134258519834634381056067475996527620254412467661189870200710444851900",
                "2784769548815220910760338722384612149549535048753605255330191563535903205491"
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            voucher_merkle_path_selectors: [
                "1",
                "0",
                "1",
                "0",
                "1",
                "0",
                "0",
                "0",
                "1",
                "1",
                "0",
                "0",
                "1",
                "1",
                "1",
                "0",
                "0",
                "1",
                "0",
                "1",
                "1",
                "1",
                "1",
                "1",
                "0",
                "0",
                "0",
                "1",
                "0",
                "0",
                "1",
                "1"
            ]
            .into_iter()
            .map(|s| match s {
                "1" => true,
                "0" => false,
                _ => panic!("Invalid value for aged_selector"),
            })
            .collect(),
        };
        let witness_inputs = PoCWitnessInputs::from_chain_and_wallet_data(chain_data, wallet_data);

        let (proof, inputs) = prove(&witness_inputs).unwrap();
        assert!(verify(&proof, &inputs).unwrap());
    }
}
