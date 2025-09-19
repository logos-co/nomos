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
use groth16::{
    CompressedGroth16Proof, Groth16Input, Groth16InputDeser, Groth16Proof, Groth16ProofJsonDeser,
};
pub use inputs::PoQWitnessInputs;
use thiserror::Error;
pub use wallet_inputs::{PoQWalletInputs, PoQWalletInputsData};
pub use witness::Witness;

use crate::{
    inputs::{PoQVerifierInput, PoQVerifierInputJson},
    proving_key::POQ_PROVING_KEY_PATH,
};

pub type PoQProof = CompressedGroth16Proof;

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
pub fn verify(proof: &PoQProof, public_inputs: &PoQVerifierInput) -> Result<bool, VerifyError> {
    let inputs = public_inputs.to_inputs();
    let expanded_proof = Groth16Proof::try_from(proof).map_err(|_| VerifyError::Expansion)?;
    groth16::groth16_verify(verification_key::POQ_VK.as_ref(), &expanded_proof, &inputs)
        .map_err(|e| VerifyError::ProofVerify(Box::new(e)))
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
                "10643309399102486957567857153106095826593795374699452172102292500155844261610",
            )
            .unwrap()
            .into(),
            pol_ledger_aged: BigUint::from_str(
                "15086725893164811954172616834548352813855877555218805048439988510537512645421",
            )
            .unwrap()
            .into(),
            pol_epoch_nonce: BigUint::from_str(
                "19641557459421245881192062599424356623307357061239367895203248247965332876925",
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
            index: 2,
        };
        let wallet_data = PoQWalletInputsData {
            slot: 1_413_348_269,
            note_value: 50,
            transaction_hash: BigUint::from_str(
                "8397461881232315868093150419720505672395990758415052842372134661984990944103",
            )
            .unwrap()
            .into(),
            output_number: 2209,
            aged_path: [
                "18323912898275500934170591165045925269817788648946832476556048643197599550451",
                "4562288795753254551810407275810515363242893311737340932277982858667246737160",
                "1459375206648687858992175198362512134950023307061866638064262892977193681951",
                "17129482638488872883140949877454247698988867984484749738892700530342777474576",
                "501749738613585766499920576011065211262650756298109228956920488516865302313",
                "19153167147637027814340924809598157931047020046203247975456976883124995096038",
                "21167116704683817754890318067547962212043330952728146858418691928121046863286",
                "9436688802660734510206842391751882178290041538035854201756556559278476476348",
                "17478882914572301808412864621321377577874218881886804645605659230875116150044",
                "16518562290772357999309753559043678705535968363838207751523560345787461332655",
                "2172926463855515505409253693975089429790132699861150894919530216555799329755",
                "4714404218871277576944239018287080521240577593692957403863247707328577171595",
                "19839797010512969103758920469464927852219405283556854442968090598818040798392",
                "3100807903487430563086352584975205880515843031456043650729051584111943916177",
                "13463874313689185549352237679274044474409201336164009399714587290691194940",
                "10275161627695824559613093557072566695807043843094409359478830708057712921410",
                "7702342880904455196841973859105619344098256486006376908982274146586995814557",
                "4988763747982068655745640430764083217463270991407847810531726242680293608071",
                "9442490762519176829509127141330820033442460992838282756820030072937270666396",
                "15377848016778673649365133009920892708910536715047753068955169138085050780855",
                "19366454020255635636428460264227383755804179367185125487930522107275198649530",
                "17964724697376745282111752313424000889480409864320211488879674433019567726486",
                "10794570607953120351898005163637807941615855618805281314172365738146173960529",
                "16165780196604840588381821203849712414744777613756526953561851783285007942960",
                "14687961660038707616073291638682797929041731928489350353872653645903599969425",
                "12725941989856021449631410869442313838605077554591570542496730020958813387414",
                "9840168203248436610828632100070979114841810089647220098180661077742174159717",
                "19215373970288776045335358655081727548601709999916639396367166446965768430768",
                "19909703732603330999012386761639339134340317215347763525456905284670032206855",
                "11107863450398615953944200789496008471690284288064805394166554129890221871079",
                "9636868433133820866153032292796836728433669756318440870912064185982617037481",
                "12445341765334743651517232204888063725854865136653549783021862022096417538127",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            aged_selector: [
                "0", "1", "1", "0", "0", "0", "0", "0", "1", "0", "1", "0", "0", "0", "0", "0",
                "1", "0", "1", "0", "0", "1", "1", "1", "0", "0", "1", "1", "0", "0", "1", "1",
            ]
            .into_iter()
            .map(|s| match s {
                "1" => true,
                "0" => false,
                _ => panic!("Invalid value for aged_selector"),
            })
            .collect(),
            slot_secret: BigUint::from_str(
                "19387126744528639841748982986171750626043947674268466286750729064186814875886",
            )
            .unwrap()
            .into(),
            slot_secret_path: [
                "9467748144701660813873906885567156742310819067194816289813895519961777051164",
                "16184043740684311124351894751366590000135085129580315711037084915212601705105",
                "627224660261650938610129095031197558307370655665936250410902739361117059412",
                "9213486098097682651844417344736844360607096569532928516628786079545321722853",
                "19165524839869966733813555105533465387809392458705324733204988555920004318623",
                "7947269959558443458568268442632275091106619808649404481528583567128534153869",
                "19190336487618896482659339494013621340361365291607229679755355530519270275500",
                "2520894669148227707504104662163789941525693750381321707198336566531614752674",
                "9108659899242494833896524344182285795289528463320270902319720373512343352739",
                "2369117938816899130104360941382838207251606801166444518816436747721910498192",
                "21771877511537040408333886339718321928979454380007759149542992675887548421796",
                "2167243885128095086814684650498942435726746965213522306592563767739512097707",
                "14115268017287608740092641291189244335082179381627399047853149951076405906882",
                "15415883195747463636424460808020570381237565067389175770842995443408337938693",
                "16490586639228374909000145942286140299072354793148143798240926490663819499105",
                "10284875151420697363016839481888166154056446425441164656597302166829622253923",
                "631167578084852070361307328529826910502127075007703721134290680320963671891",
                "13010484390148552136929236344210184047381989139461091661012999927574680504916",
                "20824064530101667124161972614137784081792812309960790896411258922319403366783",
                "463667541999266215510944499893839406576709828862504397580757024189319575251",
                "14790095539370579041170261588421932813181183539294749243568392813289180102037",
                "2415867032783144690434995933335801198675629497590543778011147659683126263775",
                "18177548958975150750399286910259217281027676277983452491057964377526850028239",
                "9251112292603489456265772650129074435124858214875418676713004632984064880664",
                "17367691923770730183020907650158743802819316139754328093141163582723237745300",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            starting_slot: 1_406_736_108,
        };

        let witness_inputs =
            PoQWitnessInputs::from_leader_data(chain_data, common_data, wallet_data).unwrap();

        let (proof, inputs) = prove(&witness_inputs).unwrap();
        assert!(verify(&proof, &inputs).unwrap());
    }
}
