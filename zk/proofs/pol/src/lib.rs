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
mod proving_key;
mod verification_key;
mod wallet_inputs;
mod witness;

use std::error::Error;

pub use chain_inputs::{PolChainInputs, PolChainInputsData};
use groth16::{Groth16Input, Groth16InputDeser, Groth16Proof, Groth16ProofJsonDeser};
pub use inputs::PolWitnessInputs;
use thiserror::Error;
pub use wallet_inputs::{PolWalletInputs, PolWalletInputsData};
pub use witness::Witness;

use crate::{
    inputs::{PolVerifierInput, PolVerifierInputJson},
    proving_key::POL_PROVING_KEY_PATH,
};

pub type PoLProof = Groth16Proof;

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
pub fn verify(proof: &PoLProof, public_inputs: &PolVerifierInput) -> Result<bool, impl Error> {
    let inputs = public_inputs.to_inputs();
    groth16::groth16_verify(verification_key::POL_VK.as_ref(), proof, &inputs)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use num_bigint::BigUint;

    use super::*;

    #[expect(clippy::too_many_lines, reason = "For the sake of the test let it be")]
    #[test]
    fn test_full_flow() {
        let chain_data = PolChainInputsData {
            slot_number: 511,
            epoch_nonce: 516,
            total_stake: 5000,
            aged_root: BigUint::from_str(
                "12507381468685418937037965643003025001770374231920403113088058676692517463683",
            )
            .unwrap()
            .into(),
            latest_root: BigUint::from_str(
                "7803303181924152474603195819210631750440756972797537055590459264425815181800",
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
                "18662412930575887120058398592311004430760497450500611284762528847372658434314",
            )
            .unwrap()
            .into(),
            output_number: 38,
            aged_path: [
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
            aged_selector: [
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
            latest_path: [
                "16395039999997733384514838779396889599139398677460652559466416505877878627875",
                "8108852177286363592580354656203175712921985635045327113980349559468346281234",
                "15287962207212123393190523628176193931870325576503730842873553605023002676470",
                "20701305536203244635873209669495724860165939560735356343316243245802678204239",
                "8061170471405146010644862412523880940864779461798050370943080231878789228604",
                "7457356178856074699685933429772876454214188101545183583916870800087708145549",
                "5134276664952366998076772738357895796134976762017768831293251595396127997508",
                "12160890460370465016506179375913103408086245054521615468896543894451288510710",
                "20190130660843086370794689706373489393071507690445499226509429586914194062353",
                "8848581117481629377088866584457922061490930340713116010828623206509169648693",
                "12416909262250521540062975360589126525877297155744753769606491736557838778162",
                "7099177463239116001223065945792415970449797276322876867075719703934010097018",
                "13676893206079329471449067238383972865677733146947193240189002289231962276764",
                "4576911884989274304723773099628488113175206095572475155294395738263957445937",
                "15154323039203668675526404785708053825282857865415963943392894126809835366891",
                "4149716573068324805016834311474968304859193723810692863752682231188410355783",
                "11314072614664268835470454805901474669204600000749994038590801039551803594913",
                "6084203193038657409062637481498966159210263582761303197993150373018416995839",
                "18090844007381793964383830932127897686539509970566841571178809435942440078577",
                "15882915585353260382550403132502916729941829057362553607958231691188183594320",
                "4437358873683358063353414565152871003128746168624053847820672599139538573318",
                "13680380879074043023910061231118166707007739364604061708019236929727310016521",
                "18508029032651745865830102051956784131579449011184684241085972162267736359377",
                "11158330120873256799560329837277078487955495715783748782745325083220203909064",
                "18808291777039145462170714199371711004727931278290730339304144623407761295180",
                "12955562149874718363090454673394698816214157524033014718500343073672989600578",
                "12566011062105422113465935999503721459025097269136314802216208273292464570213",
                "17346304472072550441757322635193797147825846037900303120589086345511381515559",
                "5948380447080246785626142205376156301767147795213142733225798826950784810012",
                "13283355256581107642074826282002859157343864682350072520858406967900398774311",
                "11076505672211448339533655648150152769550392055901854831652450403135488968555",
                "12127769390965966019322552413499429124518433575489551855209109267883912736282",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            latest_selector: [
                "0", "1", "0", "0", "1", "1", "1", "0", "1", "1", "0", "1", "1", "0", "0", "1",
                "0", "0", "1", "1", "1", "1", "1", "0", "1", "1", "1", "1", "0", "1", "1", "1",
            ]
            .into_iter()
            .map(|s| match s {
                "1" => true,
                "0" => false,
                _ => panic!("Invalid value for aged_selector"),
            })
            .collect(),
            slot_secret: BigUint::from_str(
                "14899297115557651202940060082263095194785757209823429742045168286562128840195",
            )
            .unwrap()
            .into(),
            slot_secret_path: [
                "20677273651470978733693049298481872523981906968030082171048265680298586022570",
                "19962779466205696193091038752132571193585604026955558791835257208978176531321",
                "4734318244201281823457434528818313217855948614235679393259462989710609748390",
                "58030001159461133162572348352730316887581778823709105764448739952933196892",
                "12105935220413622325542551757503743508685975764597990582359682949527067933036",
                "21366950523632637823751521846655450654245297029645424738300866852565685781208",
                "1880861920874532411901871269278201203113794533177698715899014555522401510881",
                "20829799904786448542939178041655640825800649190397009117734520268696260036906",
                "14571974259833902295286567681201232963959457379789297524899925039913790697091",
                "9711699203167677180971118566380433468833468144414197946486709976012503818373",
                "13905616317543707564503816082872969836960691541098406429464455293631149882243",
                "12386133568388183750597206184020387926048567277145471016311069787718371791498",
                "17633018993181188118575997579746648822145194104932205033437558700384114896938",
                "14427642086504923512254223914643194483054816012577912939488390735432445043953",
                "1915673523244313348500804940055885274797200285666073333160849525262328781948",
                "10264317273810899590424598672415464416247073006484280972270842804689314131974",
                "19298082030653482868998041000585538477090548185844611721694681734656516624220",
                "21526018461436956520156958178060030926125412522759693515687130629764280889950",
                "18433147516082716559518134969226298203850086845017100348992432318251032452622",
                "19967432996127051622381791725030616255175153604049368092316333362272700518814",
                "14050457841387596227904302971753403445921453025626290852044781003157197214411",
                "938598075976750219911010933658493649620132504088984236020594565011817718721",
                "4283852223894924263880379493903605211606468485923529623729874366689780444012",
                "11946685189305758778726068051454371036438615814048769489242289208978270889644",
                "18075190231587210094595199191667441133120590563713120436286305905387038278870",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            starting_slot: 118,
        };
        let witness_inputs =
            PolWitnessInputs::from_chain_and_wallet_data(chain_data, wallet_data).unwrap();

        let (proof, inputs) = prove(&witness_inputs).unwrap();
        assert!(verify(&proof, &inputs).unwrap());
    }
}
