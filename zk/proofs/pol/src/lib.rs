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
            slot_number: 102,
            epoch_nonce: Fr::from(15651u64),
            total_stake: 5000,
            aged_root: BigUint::from_str(
                "17436784089406597070717201510945703629461998397382573863051785623695184994275",
            )
            .unwrap()
            .into(),
            latest_root: BigUint::from_str(
                "8692205980044619642332206592917301406645170625490017223402329389410673146142",
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
                "12528311322899600094317627932393817086300299622536661007778805986155838486293",
            )
            .unwrap()
            .into(),
            output_number: 180,
            aged_path: [
                "8954472687336304112113542894213218877636968278910417742465315014184512177821",
                "20875050409646968571091611446449631634069558116321360454605519401589829020364",
                "19960759576143917433779305543733306870816945060376250311457143220809979130936",
                "13746017974018071866158671622010407649789233706951744881340000555371284190370",
                "6441309969323692522286939834728306622406634414196725401872291137856661440237",
                "4391066398785540955011660955510961241737304310198055539547428985206529176230",
                "13469020768078982392373430468796100664338674877747507132003732384548112245149",
                "3767051584164057354305971558357048728121028881516991303697180030085505982825",
                "11402630050326548371958110047895113011034685213596668523468924388330673508471",
                "21011633710707973570193651675164799714419159146049109772689735263355616641644",
                "20019407465343663568309528529065623582528377987465139488393076162317581396057",
                "1487414355757272540171513646668485613086998492770087434361914657162115151151",
                "3661653504594578700366177248993056689582549357078734279801152843013560480626",
                "12578692986008365938921919271073236177338508061950162605232325361340204973146",
                "8554126791736568027582192222240153574447320338751601638692429224433814089970",
                "21303555292446464897779586990880909467909332719339407677459940517090165537539",
                "7121610597060647262252231865918915329416516725227990351866184133619842371780",
                "12389255077732863242354840274111443512698331564712225390792781743930825568102",
                "6374596202641822367899752007814358824560819109391224578311772292329459225701",
                "19262678629377173986626044203579563814359274515060528892386509640117876550194",
                "9733786451411447738201298153715325046666820650152524418957652221327145697393",
                "16645943863635505057126718426397103437751556842755285330235204980961508119364",
                "15370212617469836112707139026464275189311141729615048751084315704372243314788",
                "9939985237500589569668039673082809614745111200269952937535553458619894479274",
                "21684112513016621175664098121175991894633933381146835173729307393935619986562",
                "20803768343811411429588077400588081419802470125092086646597667881220404392223",
                "14043070912611038133689242505926994966672162903432358240872446777102772609362",
                "15670741050738867913600802652219406927904588443426347151130055312384532191281",
                "13541715185798122173666053152547493270275870043649488543952113703514995174413",
                "330564054143178097382669689360531862914358889175514295666481745117759959837",
                "14140291934718500519794194356943079200734869628038653437473198605342807355479",
                "9977049026762198852517343240835437988428861863294897725453788388170143118376",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            aged_selector: [
                "1", "1", "0", "0", "1", "0", "0", "0", "1", "1", "1", "1", "1", "1", "1", "1",
                "1", "0", "1", "1", "0", "0", "1", "1", "0", "0", "0", "0", "0", "0", "0", "1",
            ]
            .into_iter()
            .map(|s| match s {
                "1" => true,
                "0" => false,
                _ => panic!("Invalid value for aged_selector"),
            })
            .collect(),
            latest_path: [
                "4157200072031500495833763496181862173642818760745958176392293483736818024806",
                "306505780309189196745879603523673851105689530863798903926305543105583739929",
                "3732588902298859046285786693631795779110146564798343634950453984190439019102",
                "21214298504811613501505386516827602071876290264897912410688955232079180493600",
                "14868835491099391070940597009274184822420301048296893195912109191356880120098",
                "771029744712510562188398390828068115955733965166844439503272942717141231812",
                "12613050652398167375398969924937355598312829727415365008677345389469130859887",
                "6719882926605427067211470134820491460456939719754679447109372917985324767124",
                "3236181637347430903677704762316321666802369393117446089548051373206177104706",
                "19576600520837139835633577344396727261497047137287226087488167724901642093211",
                "9651294130381731775082408563553482067744248556586596449357500978266069010240",
                "1194383949622750246828120118486061685197393117608226806569559361479831358770",
                "20080921829210553130052092060155494065674145932425214338439135761047860258",
                "18967502031460661560287381097656950931591579640941361806643487048077156269220",
                "18917585216004224736840086276941715723642280917803770262761493005792481735986",
                "783001228318280575630114643889756757920214743107989060164703162171985613726",
                "17604617143346291827365381173298938801371334278672041480421136169674762663639",
                "18818934666719175958417915178597078556755214782609374666527227863194442745468",
                "14498781220321110560960315926965110142023520304098457818746012846451522322588",
                "6523028402748694910932028361516459715698422053902486274152430740669407851214",
                "19390098450779711032987427394471383517689224156950146387892645258024096182467",
                "12694312889240209843768846329656927511559738779996902288213329219504301947334",
                "20297282003767286182767200752948384783981276275376267466090986191736179201818",
                "10024876579449964185368774761263862074500976127737908042820805447865763015496",
                "8333448066071805617409713925806251607383580159106058196118835157733999265552",
                "1869151096003949362526826419110968270880397441067237835182970457787696677593",
                "7329942268330805235951951218274115388352217368694690029613870776975233613094",
                "20839697692107980676402584360227394056331218880129792470624869912565616139801",
                "12992083716561542970271748582674321590668048225849800125685193476197168638800",
                "10092448880132114850397142241299611314072050977369900602966254736385331996144",
                "18966482582271513611554618890368637055112860729926063638710876414864371659826",
                "8221119635617182760040319086425128692637004012280338726389588891969922483193",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            latest_selector: [
                "1", "1", "0", "0", "1", "1", "1", "0", "1", "0", "0", "0", "0", "0", "1", "0",
                "0", "1", "0", "1", "0", "0", "1", "0", "1", "1", "0", "0", "0", "0", "1", "1",
            ]
            .into_iter()
            .map(|s| match s {
                "1" => true,
                "0" => false,
                _ => panic!("Invalid value for aged_selector"),
            })
            .collect(),
            slot_secret: BigUint::from_str(
                "12523035996379993269785332967336471484460382772639597734580159002547219611407",
            )
            .unwrap()
            .into(),
            slot_secret_path: [
                "13453110704884456962163321440942538073298086509979957604081874439040879787396",
                "10504048781170675832360207099001224723058093547668987637630684320436164379013",
                "6536755320419123212424991740146831798604527212736502683285093826488437820529",
                "2692275485579377784552765213120729670862061391921725537060557034351973593661",
                "21436598456234031203193369338464980417691740888792079222558761669026260606880",
                "12448649724841291379272422261318117020302010928079518909123472851877679920128",
                "9532252521524560278145028816356336979562750838133112816124967973660119363207",
                "9666924765130249146158566937245313310885190177559772609811110857868860876677",
                "2847460996858042022771499056691528706884297110043156558596105399939415403595",
                "6190114501261159064333532847800076043001498700893081307831859877604853896165",
                "17778511536325111358702248361961246798383045301770224100450232968992743956961",
                "9493182632055591675313981147149283597771286128824945395461323855348721667207",
                "16224990647218608456870670740744670476576211477854349597894893165495509531547",
                "2835355549520321242607837377219771870020011149637728913553757356926216648072",
                "12801778576159356297098452271491764385064798968343419417230661826822736774516",
                "5819700440149843849516664086232211662311724332985635176785856838977898192289",
                "1165744749002324098372080809105992166585743550334807943796302627561833245709",
                "16720554486043148678460243740318962499620444487913384325498836243527826686301",
                "18969793380470875897680502104029039289190769437536316309414367313174050912135",
                "4911863407042169693011413284738215140266980000745465831426915939202059375360",
                "16875091829493388227226090898781423470560212648584306671787810159747059777936",
                "19750278378587859593706359137254166682611102371587172619278567897796637999362",
                "19022876770572679092915790132850206580916498030796057828512036499392599752158",
                "13310209723562868746648898640309715788101363736530549959017275470705398444",
                "18729131887200850196725280522216060559276368511705468927840205197345522791730",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            starting_slot: 98,
        };
        let witness_inputs =
            PolWitnessInputsData::from_chain_and_wallet_data(chain_data, wallet_data);

        let (proof, inputs) = prove(&witness_inputs.into()).unwrap();
        assert!(verify(&proof, &inputs).unwrap());
    }
}
