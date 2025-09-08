mod blend_inputs;
mod chain_inputs;
mod common_inputs;
mod inputs;
mod proving_key;
mod verification_key;
mod wallet_inputs;
mod witness;

use std::error::Error;

pub use blend_inputs::{PoQBlendInputs, PoQBlendInputsData};
pub use chain_inputs::{PoQChainInputs, PoQChainInputsData};
pub use common_inputs::{PoQCommonInputs, PoQCommonInputsData};
use groth16::{Groth16Input, Groth16InputDeser, Groth16Proof, Groth16ProofJsonDeser};
pub use inputs::PoQWitnessInputs;
use thiserror::Error;
pub use wallet_inputs::{PoQWalletInputs, PoQWalletInputsData};
pub use witness::Witness;

use crate::{
    inputs::{PoQVerifierInput, PoQVerifierInputJson},
    proving_key::POQ_PROVING_KEY_PATH,
};

pub type PoQProof = Groth16Proof;

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
/// - `inputs`: A reference to `PoQWitnessInputs`, which contains the necessary
///   data to generate the witness and construct the proof.
///
/// # Returns
/// - `Ok((PoQProof, PoQVerifierInput))`: On success, returns a tuple containing
///   the generated proof (`PoQProof`) and the corresponding public inputs
///   (`PoQVerifierInput`).
/// - `Err(ProveError)`: On failure, returns an error of type `ProveError`,
///   which can occur due to I/O errors or JSON (de)serialization errors.
///
/// # Errors
/// - Returns a `ProveError::Io` if an I/O error occurs while generating the
///   witness or proving from contents.
/// - Returns a `ProveError::Json` if there is an error during JSON
///   serialization or deserialization.
pub fn prove(inputs: &PoQWitnessInputs) -> Result<(PoQProof, PoQVerifierInput), ProveError> {
    let witness = witness::generate_witness(inputs).map_err(ProveError::Io)?;
    let (proof, verifier_inputs) =
        circuits_prover::prover_from_contents(POQ_PROVING_KEY_PATH.as_path(), witness.as_ref())
            .map_err(ProveError::Io)?;
    let proof: Groth16ProofJsonDeser = serde_json::from_slice(&proof).map_err(ProveError::Json)?;
    let verifier_inputs: PoQVerifierInputJson =
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
/// - `proof`: A reference to the proof (`PoQProof`) that needs verification.
/// - `public_inputs`: A reference to `PoQVerifierInput`, which contains the
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
pub fn verify(proof: &PoQProof, public_inputs: &PoQVerifierInput) -> Result<bool, impl Error> {
    let inputs = public_inputs.to_inputs();
    groth16::groth16_verify(verification_key::POQ_VK.as_ref(), proof, &inputs)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use num_bigint::BigUint;

    use super::*;

    #[test]
    fn test_core_node_full_flow() {
        let blend_data = PoQBlendInputsData {
            core_sk: BigUint::from_str(
                "1243521580971984283788546765456513336982050553762882229075935649697333325415",
            )
            .unwrap()
            .into(),
            core_path: [
                "14899354077986736739893995913719886357387091560190878664812754607128166085688",
                "18606458877798528803522665189020473031731274309972616431619932638856992022642",
                "20204861409404779512535221984079088535435365169391730034266233535669411145498",
                "1393940010991184098827193969124622619297162482053810058093715768224412768740",
                "5792088728072490845205322012420788319813097258979745236537788376998155891360",
                "3045980054347488923551271552653480600503652931512711226728489639409359142096",
                "19259933475921485109486555985368606667919192214815196935624987054443193938200",
                "3699027954343746308804854609302926203238000795782020116149985989767670936694",
                "18240431322431484838523476254978192831222239079110526831989118670013798605714",
                "4716527931225519814357781061746264407043085468084102366150700824316535411412",
                "17089961926189150767307971644622661878737602838605802603602061449345805025014",
                "18743524919871306357880731313441845304120055919426274048778261628739761061715",
                "18664571644305177464179424814461980522661404472831340052873748133004998297824",
                "2637383115986193564895691700818821819213804782221933427824228948548954961757",
                "8332502814701055847595986924358931535480212255270924368790698529347585438423",
                "16772978970985557819415063391153649881939766259967888436611574086478313388016",
                "11113236096797589774647296616340634497336316099228449321581674554865115603805",
                "10117351169456678271158862702073261719904636287781586065834347442240061583409",
                "9737899159212679054927276862640128286163323312842892492410575747364227953442",
                "19758074506928063291541911127739980494480523814953836282436151351969048070295",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            core_path_selectors: [
                "0", "1", "1", "0", "0", "1", "0", "0", "0", "0", "0", "1", "1", "0", "0", "0",
                "1", "1", "0", "1",
            ]
            .into_iter()
            .map(|s| match s {
                "1" => true,
                "0" => false,
                _ => panic!("Invalid value for aged_selector"),
            })
            .collect(),
        };
        let chain_data = PoQChainInputsData {
            session: 156,
            core_root: BigUint::from_str(
                "9407511899939699317053206744500804221057879384131298307373363884365510557105",
            )
            .unwrap()
            .into(),
            pol_ledger_aged: BigUint::from_str(
                "801456473606247514536554589505313817337641017798795001180932230529383426690",
            )
            .unwrap()
            .into(),
            pol_epoch_nonce: BigUint::from_str(
                "20362738684904188164173875375066172826647102735682033630054721962986517191370",
            )
            .unwrap()
            .into(),
            total_stake: 5000,
        };
        let common_data = PoQCommonInputsData {
            core_quota: 15,
            leader_quota: 10,
            message_key: (
                BigUint::from(123_456u32).into(),
                BigUint::from(654_321u32).into(),
            ),
            selector: false,
            index: 5,
        };

        let witness_inputs = PoQWitnessInputs::from_core_node_data(
            chain_data.try_into().unwrap(),
            common_data.into(),
            blend_data.into(),
        )
        .unwrap();

        let (proof, inputs) = prove(&witness_inputs).unwrap();
        assert!(verify(&proof, &inputs).unwrap());
    }

    #[expect(clippy::too_many_lines, reason = "For the sake of the test let it be")]
    #[test]
    fn test_leader_full_flow() {
        let chain_data = PoQChainInputsData {
            session: 156,
            core_root: BigUint::from_str(
                "17245194574930318657496521067216150592363395809120624186030395110066629172713",
            )
            .unwrap()
            .into(),
            pol_ledger_aged: BigUint::from_str(
                "17581935479122551526433577018368482282522536639998670867733467740527259140329",
            )
            .unwrap()
            .into(),
            pol_epoch_nonce: BigUint::from_str(
                "9382732013368295937351288539099763302405932183641949806829062531873227185366",
            )
            .unwrap()
            .into(),
            total_stake: 5000,
        };
        let common_data = PoQCommonInputsData {
            core_quota: 15,
            leader_quota: 10,
            message_key: (
                BigUint::from(123_456u32).into(),
                BigUint::from(654_321u32).into(),
            ),
            selector: true,
            index: 1,
        };
        let wallet_data = PoQWalletInputsData {
            slot: 1_435_614_687,
            note_value: 50,
            transaction_hash: BigUint::from_str(
                "21065390439947646370948402545392395624652238557579612986928505993617830869524",
            )
            .unwrap()
            .into(),
            output_number: 3506,
            aged_path: [
                "2218358205308212141931945899317666000432964577322903237648485745402888847967",
                "7381568203470074517392530912421947524064215339746731564777268699934551279159",
                "21470518150682882627707907190600264878858469411047510938450953000547009809350",
                "13458519162889173366974355138961365538447048528517640420936457737491672483643",
                "11773832105663594865968339897206368837819945830164792855946953320009386894578",
                "19531531550074887068829875091417938762903199816862642611079397307940097834375",
                "21360001672176698037178744725331748977210252647687638987212076878319692467999",
                "18022043836888841206502409926421221125910821335454206803309188629420416977493",
                "17953765228261735002754058212391660814796033842452045226623135364758103004180",
                "2841365031809880034719165484167459944806824754934257037588943105121640974408",
                "20650852144806833351894937753039146471753608492579337242184558539757851769552",
                "115235042810555941967211262326418786333351585016525967856563361005177322461",
                "5496192403586143410406264749595972134166802395062583948589648609677659604665",
                "18284706634746569719876031848979115803773882127353068559597547064523532527873",
                "7610882175034460151647317497110518255758187039663704846782809723791529836071",
                "20149267738262880944371694664871739751614570604828557945900689384062098913219",
                "10005361522651612317008431557081890251420773669900050113580915541383108556420",
                "6539823241946899424284950785467978517695931377537547074806497949156278943811",
                "14436415765810302537721394641664472017874937683951427396986045105125372782186",
                "2475957069761339641035819194577819321784724614303971039274265027572407377993",
                "8395987211763106490385804283572487747601243548666740099137445887797563952817",
                "9057113843114150461689434217967772698815831077382212746811310853997215844894",
                "8524265843287892636854031187544462900418826616226673815153150566366164792640",
                "16284126253628516159985090976969832430876262840002593885197308009144180822000",
                "15498012508717561318003985041438153504559634773269917146399786455242336422893",
                "15423331394487989110531304469618728745603410601922982181979144934506189122826",
                "11346198615392784073346726083660133112961209322402632844254520692230078304050",
                "18358255402269171366955227457859649017516193184963104325560622689882922335624",
                "2136353293329823258567770315498134955710818333210321541917969621904409717131",
                "5310543577034093841892557861575792884793139299755351003030223024176419280107",
                "21747458428476877224711823467723636681057586022768686574197662332758613613498",
                "8523041650921295793627158825653387488901664707012069522898470811981302567340",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            aged_selector: [
                "0", "0", "0", "1", "0", "1", "1", "1", "1", "1", "0", "0", "1", "0", "0", "0",
                "0", "0", "0", "0", "0", "0", "1", "0", "0", "1", "1", "1", "1", "0", "1", "0",
            ]
            .into_iter()
            .map(|s| match s {
                "1" => true,
                "0" => false,
                _ => panic!("Invalid value for aged_selector"),
            })
            .collect(),
            slot_secret: BigUint::from_str(
                "11655913402027048895646069993354304406314627095091592937310845919792361208019",
            )
            .unwrap()
            .into(),
            slot_secret_path: [
                "1276120730862379103568680287024849622659106941167142009296375284364505879979",
                "6040124932443060917708591604627257324690365592745233307351729924038351160518",
                "21391376256768855350356326929068817749598671737888041013970118727685118966826",
                "21620160986093557519910862903519777863098484415358620696504957234650986794960",
                "13121175654938242994957323714166813656239023412690239155146460650579352740362",
                "19017822373899709223740751332257552378316588118373640228003361383966497233049",
                "788315235656558710787431007077999472027587487094459168797034670836925477569",
                "21186646787114616363988791516421013988529549887978531749096831413358557512996",
                "5969986544912206509829720893487939312702392818546328853434956126515798110880",
                "11027337260054719039241429381375190671927849874219073912660586232616699985549",
                "13667038539045590297083335765891215426259126699237329008543599899275400351049",
                "20054162120804988183925177280551417418491489520884359894445613089307514693962",
                "3692857639043999074814754785181395080439519882984942141646402632980930048824",
                "19259000273188792174365715640119783745652413529005073814038137831240325773530",
                "669719623422579708647910064302833094713130445567043099701479040791285963791",
                "10703923536385439442232952459987427090151310989308369216242796676091430695341",
                "14211536803822046335708413841126166904711662459643061944236288383559923470226",
                "6373450702938126682343017603767970982887816978099474806152952061239761313082",
                "12603084128770265729980929409110229568234797997127512540820828635372489534595",
                "14059591981313905736342075219085157264669071147759203074983725839631754746026",
                "4484952304651548675883183822314581417879160446809282266063702436128539321790",
                "7594650904121954414605190560064348064953515402861831165749364688623757438244",
                "13188524944690805646642525246906048450030345871450736009516816718154511831906",
                "20243856991878165897844306391610201420569555531169451154959328927811612817300",
                "6626885023949379643405388705846136814114027966345062400099364565966780572769",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            starting_slot: 1_403_099_973,
        };

        let witness_inputs =
            PoQWitnessInputs::from_leader_data(chain_data, common_data, wallet_data).unwrap();

        let (proof, inputs) = prove(&witness_inputs).unwrap();
        assert!(verify(&proof, &inputs).unwrap());
    }
}
