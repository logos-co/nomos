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
pub use inputs::{PolVerifierInput, PolWitnessInputs};
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
            slot_number: 137,
            epoch_nonce: Fr::from(15687u64),
            total_stake: 5000,
            aged_root: BigUint::from_str(
                "14646442668505825771881664659671859608636425791616340510082878133297978901852",
            )
            .unwrap()
            .into(),
            latest_root: BigUint::from_str(
                "2378923772192812289764274630025954114307650831696029376366161723260004314133",
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
                "223194124682920513386028665028808597669556304984388197313270852182195969489",
            )
            .unwrap()
            .into(),
            output_number: 780,
            aged_path: [
                "16792885221016436620771924027017742420630994430601000027938240733184696746792",
                "7162503550251400627575471996870599337040827942368920328185240779366485258775",
                "2658341744598424400120253558415955681127159089565376411844320352780081807946",
                "1203420573323943642905699911771084187269729232997455030034969610254996905638",
                "2677305301526019233171231395685077348736077230855031870345212679422437151040",
                "17031236805384039930698430787546875280876925430990612671251451056513104761716",
                "1157085733398970600915343669253123493404539082047398375486789119908828898746",
                "8373783833234827951049272868889539927877933823682070262507215598541796293403",
                "11107072660227713011097095423982654646539130649068350899317739243428248434493",
                "3357692472886828759053522114563158046764505380052597310706224692945562883040",
                "11500522195935712640179692956497683820807153575168228664917290709125682609109",
                "16169590258814904083411781840895819840722663113741241480682039390532941218305",
                "5967302031589279690057806577397165819027790146088045831191615098000394980404",
                "5481120844497735993355565067452987705183025608336748902550794894858478347262",
                "10700952950306435962571444969192991969140790279365553424705423214186630435503",
                "11371465408376347874473552083332183147032109386491279690123756287369118310295",
                "4874374697588897985476000159370252549345290329819900314072324911565490231954",
                "13720298348318195916047672433034573244800622550650114594768362617674465715542",
                "20759094536377307281516467233613981494700401934322552361593149076183583751943",
                "3011028865156145075531604034814886790910433414168436670199225190846961444736",
                "13815726241554477823634527558806873356198419991088206467862276609880308429195",
                "9341473388213795083437691009175438740730858458775136083188084235862242046040",
                "18741721313196370381986650584082698674627932022245333475818403728476624794677",
                "994321866725056626396745298523108277992300857183839755746260063381845240874",
                "13841506542813304941653792663880895765561024332110690192428269141068447227311",
                "16940895010458725650868166589587674429939656258939399328853818029491131335408",
                "2776686667763081166828064463784000530305658255732863897142244008000931574164",
                "20953248653171006903575734657024070646799133749545401303744748162766249237269",
                "14007793412447077273962603745107500985014136005727349081611129012204551551848",
                "17623201652250567002082684307006718440702581428788802123712180940771809444830",
                "18715906172604131558371724945928569349624137008887264657845200141341014700114",
                "12561093957478287246795565429146301164038980648005536420244584029247670824234",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            aged_selector: [
                "1", "1", "1", "1", "1", "1", "0", "1", "0", "0", "1", "0", "1", "1", "1", "0",
                "0", "1", "0", "1", "1", "0", "1", "1", "0", "0", "0", "0", "1", "1", "1", "0",
            ]
            .into_iter()
            .map(|s| match s {
                "1" => true,
                "0" => false,
                _ => panic!("Invalid value for aged_selector"),
            })
            .collect(),
            latest_path: [
                "5600064091247060169447350946972023159340572812822782601806167156654744548055",
                "12413841553644995058080532166730532285958558556289097944249477379524778620230",
                "529397625086491388356449359760230595459141592281568696876633145397273353148",
                "7017915236390537626143825722461323211917729745020596072195168567891967769343",
                "16009124995608495038634840846774823513900808861847100023682266809534968385416",
                "3131651036179093677267371504679205835021712523254813369779476250212224638644",
                "9683636386933578145982438729590333945350231135481373782199382707899968327817",
                "15497887495752852149251104309495149740124693022932183011583463803872597150838",
                "15589029078666590828919059060571833879736746782963482949854850628777743332955",
                "20576187368953065004336206725498661457018251538867679216802013625164676093476",
                "5175882771418158174206584249851512712987110868058725032752742558889415163105",
                "20598624896574149811479709598024707536795255452981597815197530093968091157663",
                "2321290353114255715721202759833468455500773185186279822549951702210788525330",
                "9662425476663825181196301920873651637093504010452067925643858426017342424366",
                "17100538537773929103546437634029495194895908576288237452921122399371683952761",
                "18229767057595526189271817100806599911318249629750635253776375606654303487840",
                "513510595810072180454005713205412537397617277786237477232390347740358928923",
                "10187438652886224688490832925194002780516966916133982186044945607493942202600",
                "6319200990989755752156053792709775423564649292053214275263873612724883180818",
                "16158166516485544889635851194315634772136528786671043138119919876906058131775",
                "10386416312184927668526880158997118787320584389545907828831440288573941646984",
                "9773834836503275153986051164597006061709446420663403102746652107887758536882",
                "16382554302980359609887143640190356706403411534572427797669845084766586573275",
                "13471227782017906971595385868153121137387937095609911540654648293390496076246",
                "16219556297065525192155278802699864012978663330269421661080173814523479226878",
                "4129326687450582839504268687015715399018838825393446011328424466934948108370",
                "17094902131792289996078655996005859988215072755004886995559541957164702075380",
                "12623524570409798984802185722079887231316501561832469920694251869881521459265",
                "6787887428990281263736746380741190108172837006451494951655549695968171875038",
                "1927328231678823786399525692340950597107463687998794449705956132593100619569",
                "6668930968329925338249536930701662516506207507317198501363787458848891669720",
                "6476960321520976879221520525044259819713333603355040837014398663069653917611",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            latest_selector: [
                "1", "1", "0", "0", "0", "1", "1", "0", "0", "0", "1", "1", "0", "0", "1", "1",
                "0", "0", "1", "1", "0", "1", "0", "0", "0", "1", "0", "1", "0", "0", "1", "1",
            ]
            .into_iter()
            .map(|s| match s {
                "1" => true,
                "0" => false,
                _ => panic!("Invalid value for aged_selector"),
            })
            .collect(),
            slot_secret: BigUint::from_str(
                "16087318096694206085546238667920410776138938645771667957759739076410385181249",
            )
            .unwrap()
            .into(),
            slot_secret_path: [
                "8343347179260476142038164099926305634544818425312071300596474926117631054191",
                "14649872670866582392298820232184387932646796411871039443391882074035817096873",
                "6366731922074835173331174713134738360975234577868402292782965903293526195285",
                "7486069395602331887286663738440204942678547542816677640044697154806716192750",
                "13929773982310907126697868565173457618493384871481019237162356528739225165921",
                "14144876029840186250351338109796972163849326709554882220263059387076485388375",
                "3710645085698288794095643601099896567120489562876420003518169514808067426260",
                "1895476381312101423536415280552551485389603134839477771766878662346018892453",
                "11902108308825725791839527768829868726707514141881636785260450981647713900970",
                "11066028308439796394435287595246195210015143896262101271455315459844591288369",
                "21138351879440886833751404843088048723424711630838678519921632167686432153542",
                "21195415149686518955932604822333372001617219461149557251063693055082381683351",
                "7169378426288703233425196708025859812318234466813876640522307544300623454559",
                "18101364641616788147239299646361642159510205568827096926825288692212288173584",
                "10446424428481821732902638739576045173689668466305940301233759908532387464693",
                "5297001283875780018349839085830524197419079636989345497693843444090450246892",
                "17821141541176122045069456990441289086069622334895723446913473200553099506289",
                "2266546042409836401230458907183291162393190505369507230048975029038148545402",
                "18498500508709352489382344545311267310491846296441145482182271235493160364964",
                "20596668855055256616172422606530855030375320279893139035283085175442042729783",
                "615711809466635599990120956958604416280591096066068189552765371858671705774",
                "21606487253565340186076737335570047345327643033639294332740449422041074677535",
                "20788313339202557496809693485722015086478501704742939176578929263527231975168",
                "8162243085837601224749777497073828547195172364177393114096032304765876387948",
                "15335438894368336740369596463518998883619376334153201843694696670866985879456",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            starting_slot: 104,
        };
        let witness_inputs = PolWitnessInputs::from_chain_and_wallet_data(chain_data, wallet_data);

        let (proof, inputs) = prove(&witness_inputs).unwrap();
        assert!(verify(&proof, &inputs).unwrap());
    }
}
