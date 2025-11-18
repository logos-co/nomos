//! # zk-pol
//!
//! ## Usage
//!
//! The library provides a single function, `prove`, which takes a set of
//! inputs and generates a proof. The function returns a tuple containing the
//! generated proof and the corresponding public inputs.
//! A normal flow of usage will involve the following steps:
//! 1. Fill out some `PolChainData` with the public inputs
//! 2. Fill out some `PolWalletData` with the private inputs
//! 3. Construct the `PolWitnessInputs` from the `PolChainData` and
//!    `PolWalletData`
//! 4. Call `prove` with the `PolWitnessInputs`
//! 5. Use the returned proof and public inputs to verify the proof
//!
//! ## Example
//!
//! ```ignore
//! use zk_pol::{prove, PolChainInputs, PolChainInputsData, PolWalletInputs, PolWalletInputsData};
//!
//! fn main() {
//!     let chain_data = PolChainInputsData {..};
//!     let wallet_data = PolWalletInputsData {..};
//!     let witness_inputs = PolWitnessInputs::from_chain_and_wallet_data(chain_data, wallet_data).unwrap();
//!     let (proof, inputs) = prove(&witness_inputs).unwrap();
//!     assert!(verify(&proof, &inputs).unwrap());
//! }

mod chain_inputs;
mod inputs;
mod lottery;
mod proving_key;
mod verification_key;
mod wallet_inputs;
mod witness;

use std::error::Error;

pub use chain_inputs::{PolChainInputs, PolChainInputsData};
use groth16::{
    CompressedGroth16Proof, Groth16Input, Groth16InputDeser, Groth16Proof, Groth16ProofJsonDeser,
};
pub use inputs::{PolVerifierInput, PolWitnessInputs, PolWitnessInputsData};
use thiserror::Error;
pub use wallet_inputs::{PolWalletInputs, PolWalletInputsData};
pub use witness::Witness;

pub use crate::lottery::{P, T0_CONSTANT, T1_CONSTANT, compute_lottery_values};
use crate::{inputs::PolVerifierInputJson, proving_key::POL_PROVING_KEY_PATH};

pub type PoLProof = CompressedGroth16Proof;

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
/// - `inputs`: A reference to `PolWitnessInputs`, which contains the necessary
///   data to generate the witness and construct the proof.
///
/// # Returns
/// - `Ok((PoLProof, PolVerifierInput))`: On success, returns a tuple containing
///   the generated proof (`PoLProof`) and the corresponding public inputs
///   (`PolVerifierInput`).
/// - `Err(ProveError)`: On failure, returns an error of type `ProveError`,
///   which can occur due to I/O errors or JSON (de)serialization errors.
///
/// # Errors
/// - Returns a `ProveError::Io` if an I/O error occurs while generating the
///   witness or proving from contents.
/// - Returns a `ProveError::Json` if there is an error during JSON
///   serialization or deserialization.
pub fn prove(inputs: &PolWitnessInputs) -> Result<(PoLProof, PolVerifierInput), ProveError> {
    let witness = witness::generate_witness(inputs).map_err(ProveError::Io)?;
    let (proof, verifier_inputs) =
        circuits_prover::prover_from_contents(POL_PROVING_KEY_PATH.as_path(), witness.as_ref())
            .map_err(ProveError::Io)?;
    let proof: Groth16ProofJsonDeser = serde_json::from_slice(&proof).map_err(ProveError::Json)?;
    let verifier_inputs: PolVerifierInputJson =
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
/// - `proof`: A reference to the proof (`PoLProof`) that needs verification.
/// - `public_inputs`: A reference to `PolVerifierInput`, which contains the
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
pub fn verify(proof: &PoLProof, public_inputs: &PolVerifierInput) -> Result<bool, VerifyError> {
    let inputs = public_inputs.to_inputs();
    let expanded_proof = Groth16Proof::try_from(proof).map_err(|_| VerifyError::Expansion)?;
    groth16::groth16_verify(verification_key::POL_VK.as_ref(), &expanded_proof, &inputs)
        .map_err(|e| VerifyError::ProofVerify(Box::new(e)))
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use groth16::Fr;
    use num_bigint::BigUint;

    use super::*;

    #[expect(clippy::too_many_lines, reason = "For the sake of the test let it be")]
    #[test]
    fn test_full_flow() {
        let chain_data = PolChainInputsData {
            slot_number: 135,
            epoch_nonce: Fr::from(510u64),
            total_stake: 5000,
            aged_root: BigUint::from_str(
                "18810265051722808614658023640638398154515747587007189779888983389273900417868",
            )
            .unwrap()
            .into(),
            latest_root: BigUint::from_str(
                "7259114728387323151022407770354872624525478184881823929099515020814294928733",
            )
            .unwrap()
            .into(),
            leader_pk: (
                BigUint::from(123_456u32).into(),
                BigUint::from(654_321u32).into(),
            ),
        };
        let wallet_data = PolWalletInputsData {
            note_value: 50,
            transaction_hash: BigUint::from_str(
                "4776605003479962078787925378538649339734284951912842657672872285386882760103",
            )
            .unwrap()
            .into(),
            output_number: 1323,
            aged_path: [
                "17945845305729223640013096547557321606352597925484359271833153002546521597227",
                "7208852008713222301396017733933748444059221981187705740141799198559340039221",
                "17094979533957355370430089753592716592434311553201522016362661720357715893238",
                "15958311725600450799824407631496471596454082683541669820955467383898060484614",
                "14035741354386001974674698674389010442372817015480184516242303838706813839019",
                "4165571226265216345587777912766435378716242492816310380656561702534033195186",
                "8753462203928385936728492139492525061762415493026218178874982292810377019463",
                "10646885719669495689491908147349354229035068079540551961193596034210628533663",
                "7612371264052566166024514306743063291402649184985528017396220011903250268594",
                "5929221320335065556419285034369642295937560690517807786736642666845629394519",
                "7793298336403245375479297206010454783339765845877500771630911591045985406284",
                "1214571875549490409500706728473373949169669805562456147254494053637053635110",
                "14163075670377325154115719369128701646048171627343457321237175786849787964673",
                "8298975775030954795522183338072744932560446422206781368620907840460942969967",
                "21791519227363233941792248984574053662745531261246652851069193402726238170684",
                "21181493340622539901200682691394074897030308667946856525641013839986929648765",
                "4044203393931787624484483455199743858969260932528878966084673998868301252321",
                "13501320550184229377871307688641443640655388439126153683898707670924157234747",
                "1903011438729466341881519869883398369820929764513494055534931882287443704840",
                "13184123574203709880739171107635858911680178606678062718936258834795223475438",
                "13125700501983551846436862320794432969973795988394657956236628224222606826020",
                "17899689967307517235861558433667611522238756244610521038965158609628973518364",
                "15694734374826475525631022945386984889833297633835094613827690312847912444454",
                "20526607775952964762785502907783843138097513525986397227549603593107397473096",
                "14110852806042157018730648474021227808092362312245934233232480835407993172319",
                "8532705361667408856376539265854434171720256707761942188425538029681017562407",
                "2741583664283388200418311955560361390212635269820123415856154436592064566848",
                "2963959896965640955391106307185016648277257522066447253898195608971923037926",
                "17292580729942988365685579386210309202295718456656815597463837082076606732866",
                "8034620609415826332053543086590435123363663821519643558130216632606205612412",
                "3946044984643855985940193925182898874022638276669074648652214885451111817031",
                "5836001448678372474875958164083984712355351754975581715700431745610481250444",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            aged_selector: [
                "0", "1", "0", "0", "1", "1", "1", "0", "1", "0", "1", "0", "1", "0", "0", "0",
                "0", "0", "1", "1", "1", "1", "0", "0", "0", "0", "1", "1", "1", "0", "0", "0",
            ]
            .into_iter()
            .map(|s| match s {
                "1" => true,
                "0" => false,
                _ => panic!("Invalid value for aged_selector"),
            })
            .collect(),
            latest_path: [
                "11375763441155508752940249570321034169939764824261348460374576782241370002784",
                "7617695534419281335152624811529699026802594778094831760332409738606802696355",
                "17749532924708203642685455783017720761161322812768333550442408892780047745379",
                "12798183598630390742600699393207708710578422045831739951526869571794788757966",
                "20566879395564889929124402025661207858739602857232162683294733003545779379837",
                "7196342392330097484861721791355526028193242125620031940519288860008906025081",
                "18864598244570104185928078115761338632642886516394185834259164195213080668978",
                "5657057556860394182246472855142483676999432635259965200572150835973048882898",
                "2437172866687142144664121381232251424157128336192694934243637254304207160432",
                "20047587909480075528890101874440452438040909777263093731753091660415805838028",
                "17437238432239207319517782393926375085995328158325794573381751024538081872221",
                "15233747664715553295479743016644690270415381884893072715314626595312437989024",
                "10932625667563032525166023252535894920101898026408250694269371076794043395435",
                "13002527851974501852167559973408869306451199177800691312728984603906429021440",
                "4470686583679698303469812693178589956294295389756372717866660665788998547947",
                "11908795059330318128886657026876465177689146317525930267633489561179522179925",
                "7796092147849757842397989472654994051335531863699978376077134331836803087328",
                "2722901665049709181933756829462853852454314487995905935263251445311265647585",
                "15726693054301616766313952095171320301876499286045781603871054105935351014733",
                "4721766579930951288965599633785125684590576620748026203032505052872556024532",
                "21199632379198603337799998521179564929519710389229862770241811428656841547554",
                "19757309305693083672712647304464070386627991034603525411941485299990508529828",
                "20851425147479511136069193127327958140362579761786508346033753349889885405235",
                "16453680907436286522479788522801532500190592671204859038638198284008614064158",
                "15032134556826536203601169690754463926639964307154382559477598015353403408562",
                "4889015486956980355744665891547980678684313496885731796937731559728493392446",
                "5494797522176788150133670992481671301963023859741545325948378868508103307281",
                "1941903537224442418673400196996024055440371392924860226677498102052626097949",
                "17348141702300434420574132776255261445872690312608615846613494897297419997241",
                "11044804465877726556192915309902137532233303544688301152317291943890442806821",
                "3631162661698060677059497249753048468692940578616273263937763786858600253890",
                "12606573906297847880905963443113778147663765662413278724505402117427755073391",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            latest_selector: [
                "1", "0", "0", "1", "1", "0", "0", "0", "1", "1", "1", "0", "1", "0", "1", "0",
                "0", "0", "0", "1", "0", "1", "1", "0", "0", "1", "1", "0", "0", "1", "1", "0",
            ]
            .into_iter()
            .map(|s| match s {
                "1" => true,
                "0" => false,
                _ => panic!("Invalid value for aged_selector"),
            })
            .collect(),
            slot_secret: BigUint::from_str(
                "966290241215509884694116352266383523800720317049020468247678415322461821651",
            )
            .unwrap()
            .into(),
            slot_secret_path: [
                "16967706338438051091517520838877290000829618531369060439314676366506921735252",
                "17481309197154591130680239810591180524686185137817890393867688428098568279464",
                "20269683235690581590525254534130791828991263655601997167322286201725720941905",
                "2358482118775482575254473288812953032496220795938286074609867523672158351362",
                "8850535754188052326704042467778491012103317573319595758901586373848037766756",
                "20488023733515841845011268325392026726863028944809524362823742716898408701123",
                "7180404093639718966925035674055658196147562154868563158986349776634765020756",
                "4318257445109632690032736308174793662647277003440394462698382829841325814169",
                "18214635080079212917795931598131431548976418832825952549735133671203455046440",
                "9798767666402991203021820892592167524126242919406174531500117937091589732644",
                "6041589811083264099290030054044760851908770618366771256317748508871691012078",
                "2396017002009482528372356688296160900748366481327652259196278383680694228469",
                "19025478304759374824890476211005759264218893983998171171867175476161973941730",
                "14636715766887849339227720783880072908074278839794775566830805774272512702199",
                "4074693834227804114893150736374403274727136710572189911107049449987691723306",
                "4301140894230753775607576522990216822846756121346495150385445718356272915140",
                "17989922909398937344145522310261601274628421904522054482093530375017349570766",
                "6321014771828903367814825175478193693231562261576972852901493830477981108645",
                "2819819438712215509626584002325494099411524280392915686336676063487905875012",
                "3801571691209039220723097285710311804530923576657242928481177833626086368795",
                "12833400476032487368990386932334207953530056434754988461256133608847441377401",
                "12102781852589788235366509974230179361217625107328632183068859925481185452155",
                "1676117727797126383187738747972645903126253678217594200568008573690210791803",
                "10514744970157070547782822325450699314459587193118365900526284974649593156325",
                "8688249037267567371155322974722214337189790046127986693493463689043222370125",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            starting_slot: 116,
        };
        let witness_inputs =
            PolWitnessInputsData::from_chain_and_wallet_data(chain_data, wallet_data);

        let (proof, inputs) = prove(&witness_inputs.into()).unwrap();
        assert!(verify(&proof, &inputs).unwrap());
    }
}
