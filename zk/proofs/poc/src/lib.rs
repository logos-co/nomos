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
                "468499858532566301463914075708835803713595188142229287054972131964200960617",
            )
            .unwrap()
            .into(),
            mantle_tx_hash: BigUint::from_str(
                "6385877842415093593703722460129495869851985946873010858344790403230571356558",
            )
            .unwrap()
            .into(),
        };
        let wallet_data = PoCWalletInputsData {
            secret_voucher: BigUint::from_str(
                "879270297141153048020242862287246513371477074360824038280322576992571874639",
            )
            .unwrap()
            .into(),
            voucher_merkle_path: [
                "20402003273120574470330190497336326791048237433382985303393893459630420668043",
                "12403917530130933589063386895491187749633900679279624162070730626981270590772",
                "17290174430627866614549770812020849090566389956540356436358236622404117176979",
                "8483734858601856509633370648180808021800929275127490847842983628364353547540",
                "7278935053778510871641192717216962562446562589371789871578796331497787840608",
                "7173998711163287931891003192750250544193640745174667763414705149660632452141",
                "15323039677453671339714712831553645913640704337854171471238999397469450471106",
                "5298147751657438645609090207830770149314985015035945556017995948220298603457",
                "8076044951063674643466413606516490480352336374114344704121845632506723172635",
                "21544913937505758297115138987523934926632849735277963532891518674233850909721",
                "9912868485159843090333734158091762439481023826672648516809047015690965142922",
                "19413671247392965742937923554140593787461027256046470715547194771603133714327",
                "18076802211918124849487080734714967394693080784221606589053505163632124491271",
                "11584090098135138812285216867305009171193018336186022309648393214481919668500",
                "15747508200793392578339915239435073403814241962915745153199063726555383452353",
                "5908907016235420356703492237789230447589705762094252281329132272148934191601",
                "3469412078336674938330831991295795808074549844319084535666036692508834272564",
                "8927826393513627889680178020099322358183337845389604211476382790857053684824",
                "7343725523499180146302059358370572457753857062971801708368489063522274626549",
                "4869398368028192302509015666020979811873097352583077991839885902967968130064",
                "8213009993926111034283749125421177426799836491429579218573072172041037596031",
                "5936007845228548592086310899406802939255072809456895407079436858030398968716",
                "5611586041268171227848489275036513310802657485115027643632960944513120697395",
                "4346336260858872805341011078157641702533463534268475645908771847414102779262",
                "10817753691061145918928325181684731882162634774434126435118677345380588967772",
                "20704817036189378478319655665107712550076556162955655927801971501979850851771",
                "6404977734996062091249736142489508403863267805788807467114143910863815814615",
                "63065908081546860111845307110646263891028000660556138259662886966363543849",
                "4043965391406094968596739491046194517232450765074486566772647374776240990846",
                "7683985629973528450031440499854348588819261669336029248458717635100208433511",
                "2782387032743021329561911147867278681212755613349310866701333507151029602172",
                "3345190031263577129343733711056244817130997447505323279513931497706009969411"
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            voucher_merkle_path_selectors: [
                "1",
                "1",
                "0",
                "1",
                "1",
                "0",
                "1",
                "1",
                "1",
                "0",
                "1",
                "1",
                "0",
                "1",
                "0",
                "1",
                "1",
                "1",
                "0",
                "1",
                "1",
                "0",
                "1",
                "0",
                "1",
                "0",
                "0",
                "0",
                "1",
                "0",
                "0",
                "0"
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
