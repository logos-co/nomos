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
pub use chain_inputs::{PoQChainInputs, PoQChainInputsData, PoQInputsFromDataError};
pub use common_inputs::{PoQCommonInputs, PoQCommonInputsData};
use groth16::{
    CompressedGroth16Proof, Groth16Input, Groth16InputDeser, Groth16Proof, Groth16ProofJsonDeser,
};
pub use inputs::{PoQVerifierInput, PoQVerifierInputData, PoQWitnessInputs};
use thiserror::Error;
pub use wallet_inputs::{PoQWalletInputs, PoQWalletInputsData};
pub use witness::Witness;

use crate::{inputs::PoQVerifierInputJson, proving_key::POQ_PROVING_KEY_PATH};

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
pub fn verify(proof: &PoQProof, public_inputs: PoQVerifierInput) -> Result<bool, VerifyError> {
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
                "7135799469236250618701431393244478925368198276928704519155517568579689388721",
            )
            .unwrap()
            .into(),
            core_path: [
                "13215962979733425800842556765849875526375515558888350407767418397026597187563",
                "4204511935479507127547792588787528631914098888999629897618862337116026174256",
                "16809956839393676327761493957347468030307479224829885825121946655904165565145",
                "17767283197395036343850743212584523685322094621190090677939239941996608736913",
                "1513806959387340552718111162700673661060844906372746129682878442179537295601",
                "12915473776249528041058872511121340358976102167258459949890656082219471691654",
                "9198861181498324387765478828826184543046241838917624586927964795570993239414",
                "11151880665697153547440970291047406710373635831712560117532180002596812031695",
                "19773101698311911005560265892402178575067950164370631742267844078461572531317",
                "20988833327883839505414689591247976162638225437627205483067829853978542612529",
                "12618616338019061298565189586400894095084808809743133388097782140803111531719",
                "4654614479923741460071255894813480007553579611171293646330491063892935067829",
                "17991906201641317135211421316471892946022563090302041093801209810513314164935",
                "14406642360146226492125045121710983363976888701467125309582814723700433466331",
                "14356229949162206585250759430838631534123020604399862327585819124443734903396",
                "4882298859594365890136288786509057395982037127292414515331611334806546584242",
                "10262608032089978576370861822797718781470648871033633613798499965236970129357",
                "18531206352150831458656908023910744884690077366273310774565262588270481379873",
                "20091696623051588466384798957365977541814875145402541262489697388735255401301",
                "15347260640730291721070246150272885134042670739518305089940205753076161292203"
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            core_path_selectors: [
                "1",
                "0",
                "0",
                "0",
                "0",
                "1",
                "0",
                "1",
                "0",
                "1",
                "1",
                "0",
                "1",
                "0",
                "0",
                "0",
                "0",
                "1",
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
        let chain_data = PoQChainInputsData {
            session: 567841,
            core_root: BigUint::from_str(
                "14663954094209697841179506274221044382912377258592971571160012092518377785731",
            )
            .unwrap()
            .into(),
            pol_ledger_aged: BigUint::from_str(
                "1868648733800877348872373365176874236386998448789764385509195113963471762768",
            )
            .unwrap()
            .into(),
            pol_epoch_nonce: BigUint::from_str(
                "11311658591014757367914230804243723060485835174075055983451892717423067677568",
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
            index: 13,
        };

        let witness_inputs =
            PoQWitnessInputs::from_core_node_data(chain_data, common_data, blend_data).unwrap();
        let (proof, inputs) = prove(&witness_inputs).unwrap();
        let key_nullifier = inputs.key_nullifier.into_inner();
        // Test that verifying with the inputs returned by `prove` works.
        assert!(verify(&proof, inputs).unwrap());

        // Test that verifying with the reconstructed inputs inside the verifier context
        // works.
        let recomputed_verify_inputs = PoQVerifierInputData {
            core_quota: common_data.core_quota,
            core_root: chain_data.core_root,
            k_part_one: common_data.message_key.0,
            k_part_two: common_data.message_key.1,
            key_nullifier,
            leader_quota: common_data.leader_quota,
            pol_epoch_nonce: chain_data.pol_epoch_nonce,
            pol_ledger_aged: chain_data.pol_ledger_aged,
            session: chain_data.session,
            total_stake: chain_data.total_stake,
        };
        assert!(verify(&proof, recomputed_verify_inputs.into()).unwrap());
    }

    #[expect(clippy::too_many_lines, reason = "For the sake of the test let it be")]
    #[test]
    fn test_leader_full_flow() {
        let chain_data = PoQChainInputsData {
            session: 567841,
            core_root: BigUint::from_str(
                "11919727315470393760956730772583757470829260348273048807525679337050662173914",
            )
            .unwrap()
            .into(),
            pol_ledger_aged: BigUint::from_str(
                "10726106912532386783432057391734903282929386606897883762297740575888409024455",
            )
            .unwrap()
            .into(),
            pol_epoch_nonce: BigUint::from_str(
                "11489960498392973367532348440372433465930661452782856847122493462930895430188",
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
            index: 8,
        };
        let wallet_data = PoQWalletInputsData {
            slot: 2_571_846_702,
            note_value: 50,
            transaction_hash: BigUint::from_str(
                "6768709679174384532197861166948469838711726777027371811443147093118820882332",
            )
            .unwrap()
            .into(),
            output_number: 3370,
            aged_path: [
                "16573634236960536865147646395608874530395919418568692784433411221132191442671",
                "7670708299730485280480697156529929353960323424437586541929559406790098244766",
                "9070246143769919473621385380905465793300735879059350033958116823494561632917",
                "18723384471762245943924678992659314494439799355675639071619552291938875630438",
                "14237084539167552887347147248361819560529389091657710185352901835237711902558",
                "5726624521233577753176994525014465541168183475874751541897609852460722688854",
                "19861067380024347060447420197765175261685079488294018260769423158931472048821",
                "6917015551298526472198710230039306003598567108493044986343752513252824225148",
                "15273425044206750936752391910159726128928440946553268203818906939173427316439",
                "15044019195251908455717261853593428825492801208098944393943175717313332575350",
                "15238509927435795753758604730567361350349572644077511404318701204969385097946",
                "17807371068528460838641668486739577003193829663622981231366156508277560398858",
                "17278900643349400358428336826931739137053569494153064468711767549434362486481",
                "15832865922983921746446031757560029024756269151495930947610782188557515492281",
                "222505747088352137449433908639240244421942354962093324125545401880314032604",
                "2402179119457992511578591788889331310174570549995849470227538747635348635478",
                "2000440001311365432647069803578804619597523106467069827452561289883268677863",
                "8925537647837677581587556713104537958319044358483763192871459317480072306932",
                "14412100745597691923253066063527260132933283299317686220047996244889346219899",
                "18119149395374164611711654095460934715837789649629409546583318015122215535776",
                "10446341022037337907256913889187343932723958001444703976101401021725262699919",
                "3035671212212839651218269004906257359309181838168644543632897724583930598447",
                "2220276801349295097042576255185522862337349167357843961422984129336906870543",
                "9712056337309658319179188516256637100701983828826182527296937074434129986141",
                "13337110268804039199090519860280643453220053895426234839427793512634832798670",
                "12029440234194624052558247440057215559373991215129799245369168260118360708403",
                "20766449601548031022109279829638848683384264044031184371418787131546822162799",
                "21416175494884522259468312279718292657055043837833951508849764243510636607874",
                "16239965655036138447193222506445610491395296853289602530595886354101553441612",
                "3043937613809453573323851773767471834647407770987584290124039449440486271380",
                "16965754564672326742454328124526525738968318825375108030722275779074104823169",
                "15915203529038229733273392942218624331954004332331146058196405559972450421337"
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            aged_selector: [
                "0",
                "1",
                "1",
                "0",
                "0",
                "0",
                "1",
                "1",
                "0",
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
                "1",
                "1",
                "1",
                "0",
                "0",
                "1",
                "0",
                "1",
                "0",
                "1",
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
            slot_secret: BigUint::from_str(
                "9385314168854376345110108494916149894869390393850785486632595789114852868431",
            )
            .unwrap()
            .into(),
            slot_secret_path: [
                "13879977871555763824687532553718024319625545589505842715463614019166256870238",
                "1994365824643335777318190745530884354771201736130211111553156529101943603151",
                "4690618435646297280069796643728283327843704832519333887148416412005591477607",
                "8307930544868420602936048226434442687751946658336944516732194228323286549567",
                "21416812454747883242416334082826345560148065520314133307590428536738242024940",
                "20788926963294412884583939441506256164021616241206202227331073124038735585405",
                "19463864166923140217363274261456961115421871164319475789317728887620137361125",
                "4438184972308684886466594909748035060365479359445949291771867347086639820745",
                "5323091863653726434861475384024643421753988671838248498350682291669270726917",
                "7438803048040341473066095071897238520705952045286846145958650849782382720541",
                "3673334185696502924516742667450359441655626583720365553178484078712141313756",
                "14200184579015208172407745924949748331556478880968336193187437572188148551346",
                "18291569928119263302666104730369938179832473177248989334280938342330628861649",
                "20912809121735595917662587076822852234216296822625285593206222623034952373609",
                "456383780109771929804762152073024532987568451799584194751620479774787405883",
                "4600267996786964145905017722679149136874560513130662497976193645342525457754",
                "3509972693245418161030107420562445177953822814564128725149177612751434798181",
                "16912248879095847223607204125390865014886898379323132321566019279264833513415",
                "20839552317793785570684590210354108777881262387929802369507218385290146825954",
                "20646163721352211184031460568450549483822111378787072319888988768855263507301",
                "9270286322469945745982673038714323367810232586503511612573313777894401639272",
                "6547926712228064050857445158427168670156072083565661379890560066562403818063",
                "15801410825398401801420771262228312582410895852529622538093581088952498106038",
                "7475166950799495778989690396703340200703707217691786255936412879676754587562",
                "3489949363949353595984463806359011336215576821535916973708969287993588167929"
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            starting_slot: 2_542_712_468,
        };

        let witness_inputs =
            PoQWitnessInputs::from_leader_data(chain_data, common_data, wallet_data).unwrap();
        let (proof, inputs) = prove(&witness_inputs).unwrap();
        let key_nullifier = inputs.key_nullifier.into_inner();
        // Test that verifying with the inputs returned by `prove` works.
        assert!(verify(&proof, inputs).unwrap());

        // Test that verifying with the reconstructed inputs inside the verifier context
        // works.
        let recomputed_verify_inputs = PoQVerifierInputData {
            core_quota: common_data.core_quota,
            core_root: chain_data.core_root,
            k_part_one: common_data.message_key.0,
            k_part_two: common_data.message_key.1,
            key_nullifier,
            leader_quota: common_data.leader_quota,
            pol_epoch_nonce: chain_data.pol_epoch_nonce,
            pol_ledger_aged: chain_data.pol_ledger_aged,
            session: chain_data.session,
            total_stake: chain_data.total_stake,
        };
        assert!(verify(&proof, recomputed_verify_inputs.into()).unwrap());
    }
}
