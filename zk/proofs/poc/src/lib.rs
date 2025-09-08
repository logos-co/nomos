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

use std::error::Error;

pub use chain_inputs::{PoCChainInputs, PoCChainInputsData};
use groth16::{Groth16Input, Groth16InputDeser, Groth16Proof, Groth16ProofJsonDeser};
pub use inputs::PoCWitnessInputs;
use thiserror::Error;
pub use wallet_inputs::{PoCWalletInputs, PoCWalletInputsData};
pub use witness::Witness;

use crate::{
    inputs::{PoCVerifierInput, PoCVerifierInputJson},
    proving_key::POC_PROVING_KEY_PATH,
};

pub type PoCProof = Groth16Proof;

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
    Ok((
        proof.try_into().map_err(ProveError::Groth16JsonProof)?,
        verifier_inputs
            .try_into()
            .map_err(ProveError::Groth16JsonInput)?,
    ))
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
pub fn verify(proof: &PoCProof, public_inputs: &PoCVerifierInput) -> Result<bool, impl Error> {
    let inputs = public_inputs.to_inputs();
    groth16::groth16_verify(verification_key::POC_VK.as_ref(), proof, &inputs)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use num_bigint::BigUint;

    use super::*;

    #[expect(clippy::too_many_lines, reason = "For the sake of the test let it be")]
    #[test]
    fn test_full_flow() {
        let chain_data = PoCChainInputsData {
            voucher_root: BigUint::from_str(
                "12507381468685418937037965643003025001770374231920403113088058676692517463683",
            )
                .unwrap()
                .into(),
            mantle_tx_hash: BigUint::from_str(
                "12507381468685418937037965643003025001770374231920403113088058676692517463683",
            )
                .unwrap()
                .into(),
        };
        let wallet_data = PolWalletInputsData {
            secret_voucher: BigUint::from_str(
                "18662412930575887120058398592311004430760497450500611284762528847372658434314",
            )
            .unwrap()
            .into(),
            voucher_merkle_path: [
                "10537658014734644936558954312251569282270651863172095594883544299035848445588",
                "12067929117387767164899481230071754423875115491947482167629846914462270972647",
                "8865977197532726912208341825423082928440635625444701809041577908252459240113",
                "12018509870995309283923110911830285363824326473211494826605288859318122619689",
                "4993751033454016613567270858241605110100415690118714639548053987337198480811",
                "4967427698943461078767776265588755194714289020997074945342142622709924316288",
                "5993831779965993650346672044672904108192432596217829566706083713267612163361",
                "5421098556696349684919722365394007667403512794225192216078028222023294818724",
                "20458468676630197680309502571017877438046975625697662842782345862157950744013",
                "3473975385756330703685611242812640352277563688898208673272411245421660556460",
                "3469027953362776710714455934766774938254292976083226217047010473004144596774",
                "4792278055311870118380580320031948474499185782632632393590803992276817748957",
                "10320344111762509029563791494437978839749528492807546135437534592754971206508",
                "21625237938222292257085830582111136637647594052784925188971850897514246979870",
                "3697462783491799888479336946536757673523848945947748799366431856368513999179",
                "16166531437488481810527111158463356441932623677578980405772516668749502992777",
                "4851444230002410188614124531765769241656936675580731411135609793563321429095",
                "18128815355431288127031329056639722425314446241434680158768777532497391933068",
                "19016457564048669673826411076383372975637813761748849349252558188158804342371",
                "6963313557055788395982447206416917377104117450454761603310774030042492885967",
                "1420865563251736146078861129000632920178924157206653163604864675524830643194",
                "5091218127927385648298671659623341115648155524132833149623229066111569041717",
                "18714503906603232997660644232609104249509714545123582437144078521496457820533",
                "14418823039151185068359943045978096402585557971451259493651481321349647253103",
                "27619380745939943988191573031745813564347121501715955062711984458630360720",
                "7084817958108522519183190792769679028619484066921584043710544357411569209587",
                "14241678160147536917806270268809623582721330661385921889353874476364312065128",
                "20756234994153334465243980974216911807246143221565853085279935003244456510876",
                "4206517877314247009631097050460625396184519599116527556331004382943026228541",
                "14127850435485937331920301503476846468217283152713130518135737739439644382031",
                "17807744231334778720123704406785569034895042215156248970227657225725098895548",
                "14396348302017710098633448046268667836328174667243058545373772626151877550022",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            voucher_merkle_path_selectors: [
                "1", "1", "0", "0", "0", "0", "0", "0", "0", "1", "0", "0", "1", "0", "0", "0",
                "1", "1", "1", "1", "0", "0", "1", "0", "0", "0", "1", "1", "1", "0", "0", "1",
            ]
            .into_iter()
            .map(|s| match s {
                "1" => true,
                "0" => false,
                _ => panic!("Invalid value for aged_selector"),
            })
            .collect(),
        };
        let witness_inputs =
            PoCWitnessInputs::from_chain_and_wallet_data(chain_data, wallet_data);

        let (proof, inputs) = prove(&witness_inputs).unwrap();
        assert!(verify(&proof, &inputs).unwrap());
    }
}
