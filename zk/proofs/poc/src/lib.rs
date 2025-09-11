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
                "16128370659585010675011771178833344032000886669733757231936455548689048492407",
            )
            .unwrap()
            .into(),
            mantle_tx_hash: BigUint::from_str(
                "2782986857467528388935427084192832749498469459298277895337277564155580291177",
            )
            .unwrap()
            .into(),
        };
        let wallet_data = PoCWalletInputsData {
            secret_voucher: BigUint::from_str(
                "10793085795916138527223861035816627613642352747244105816111835772186196906919",
            )
            .unwrap()
            .into(),
            voucher_merkle_path: [
                "6337994124918675554726155135256794004058201148752066897418152184792742239940",
                "7940696123991997739433594587477062905873923771864145801194255161745694322193",
                "1194070381079910704643185378111458641171449703890417738069055777608151068933",
                "2564255306139013562760265175036243471387970640956084763109910417619634569560",
                "12504464122794598723823578275555386994533209214853582226868022694154921889584",
                "6462477780884271285245614910742403767586668597225932004511949411417010823702",
                "2055326497407807095864000001650841225609048742060640920552033045526907388100",
                "3132438041730321741707406393533271678930282112239449870601737456697529843752",
                "7119142255394224726051862185331816532540065141219695632538581513875896962740",
                "7126814529892049368721839325201171185034292634281777664949753898541958108212",
                "8729489387170378374045987361798927820589423962713981530658637531433844200115",
                "19024949288019900607693146782383592968794056432527450075422175686864496230170",
                "6140304674101056456376058821211026061388767276934558753066640897020391591706",
                "20404843603878928762374436336977292246250404951479009937273018521313092855413",
                "19991016677581751579788633304790968075976800588814930554973259598733926311580",
                "8260700803914102997483225310222938204896608317438681374033079735188379683119",
                "8249974496462481532662652642054156910628339825787550746181421402891268610735",
                "9949781462952804061420305900820209250906894316657376146995268681541822618827",
                "19393887553530879747437800805680748281128140043180390273811761485863561244843",
                "19730164937812155156074948470965108846330150252209073888911690608883719066773",
                "8737512482258143854242366314007196588066364248268285158451022656215164535067",
                "14620422660970928993943202287012658936323009605706019437619854208999438641228",
                "14899401850866938880270928325816863980122672778695750268520723317951951339802",
                "8067333228519309233020151210433926006989438856292963120525521635366220720423",
                "4053362534198003069363840095012304065653195960557834340652836249776099245426",
                "18014285182646335392013202888744653909844361023144181309561961300196995152426",
                "15876067041619109212877584173051564329359555949861962779924354206453398855095",
                "13398614998271084057767556453012435316028378614983516300158853531957204974539",
                "21650400238100003290282358382136900504079655201999300087947502008158506393413",
                "8615723691683063849101742194005806906280276935520755598556283394140430378432",
                "13611761874585758178104581413575288298270490680357070096453835734452221864585",
                "3170260668713674390668153320356339569410757856799668969439405488227024544476",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            voucher_merkle_path_selectors: [
                "0", "1", "0", "0", "0", "0", "0", "1", "1", "0", "1", "0", "1", "0", "0", "0",
                "1", "0", "1", "0", "0", "0", "0", "0", "0", "0", "0", "0", "1", "0", "0", "0",
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
