mod blend_inputs;
mod chain_inputs;
mod common_inputs;
mod inputs;
mod proving_key;
mod verification_key;
mod wallet_inputs;
mod witness;

use std::error::Error;

pub use blend_inputs::{
    CORE_MERKLE_TREE_HEIGHT, CorePathAndSelectors, PoQBlendInputs, PoQBlendInputsData,
};
pub use chain_inputs::{PoQChainInputs, PoQChainInputsData, PoQInputsFromDataError};
pub use common_inputs::{PoQCommonInputs, PoQCommonInputsData};
use groth16::{
    CompressedGroth16Proof, Groth16Input, Groth16InputDeser, Groth16Proof, Groth16ProofJsonDeser,
};
pub use inputs::{PoQVerifierInput, PoQVerifierInputData, PoQWitnessInputs};
use thiserror::Error;
pub use wallet_inputs::{
    AGED_NOTE_MERKLE_TREE_HEIGHT, NotePathAndSelectors, PoQWalletInputs, PoQWalletInputsData,
    SLOT_SECRET_MERKLE_TREE_HEIGHT, SlotSecretPath,
};
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
pub fn prove(inputs: PoQWitnessInputs) -> Result<(PoQProof, PoQVerifierInput), ProveError> {
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
    #[expect(clippy::too_many_lines, reason = "Test function.")]
    fn test_core_node_full_flow() {
        let blend_data = PoQBlendInputsData {
            core_sk: BigUint::from_str(
                "3013202771936692510070072983700659453481703097604134746268405519189665464555",
            )
            .unwrap()
            .into(),
            core_path_and_selectors: [
                (
                    "16709913420418130248257618085780626447540584285349714059478883323918798123631",
                    false,
                ),
                (
                    "9144930953728030331967217779399566210237930800334072709853296528435817531642",
                    false,
                ),
                (
                    "13061261593604692501214966454824582306179063323610803160772896965517728789443",
                    true,
                ),
                (
                    "10561776070484459371883856617868155628906741205607630105933303389420446195512",
                    true,
                ),
                (
                    "9336514793350650165965348294771965381646738575860959741360499228330849985184",
                    true,
                ),
                (
                    "3515678751472231766791943680160413650070418395846878932802510770782300907138",
                    false,
                ),
                (
                    "4258927015651753890054996563212457355600740454983630299280705416402954445404",
                    true,
                ),
                (
                    "15544663450153047036307176270893467266383053919829631030606101578753180720903",
                    true,
                ),
                (
                    "15523118484405522172917495946079144156146310785401640990730798970767447076856",
                    true,
                ),
                (
                    "18034650434219193894438531844461722373517747000360160256083603493625996069998",
                    false,
                ),
                (
                    "18995593979023524047484341577429783567334988663640079410877624661140936269162",
                    false,
                ),
                (
                    "2775323711292885982322199821763485164454011304615378251672614282850377232939",
                    false,
                ),
                (
                    "10134457793737296083214319677790047822327349516636596806321994072890392441015",
                    true,
                ),
                (
                    "10058178679098882266161615642706849865520650625610428057361725665286880878976",
                    false,
                ),
                (
                    "875391660197308796144610235177467277518737387549466689719343861616145483383",
                    false,
                ),
                (
                    "9863148119352229885126678344268391480674031777481648469556859415975901276788",
                    true,
                ),
                (
                    "2043070611506395100671958996426625851184876075118633633206855357743918235312",
                    false,
                ),
                (
                    "6563911172354143607355752453565999282675554867084110392615216024631192710325",
                    false,
                ),
                (
                    "10541341733459577933847442944701877757152836241437076151981380008697122580819",
                    false,
                ),
                (
                    "15147839191417198229397210779442072261360361405257106931423966513368652537129",
                    false,
                ),
            ]
            .map(|(value, selector)| (BigUint::from_str(value).unwrap().into(), selector)),
        };
        let chain_data = PoQChainInputsData {
            session: 50,
            core_root: BigUint::from_str(
                "17555823411982952543742571043427936847778948408715986938502143044007568238929",
            )
            .unwrap()
            .into(),
            pol_ledger_aged: BigUint::from_str(
                "21240551203239381668079527892033085215042376883502339142561019249525585404826",
            )
            .unwrap()
            .into(),
            pol_epoch_nonce: BigUint::from_str(
                "17194717435243199065908451230918022154018645781443876577330139376177315675705",
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
            index: 7,
        };

        let witness_inputs =
            PoQWitnessInputs::from_core_node_data(chain_data, common_data, blend_data).unwrap();
        let (proof, inputs) = prove(witness_inputs).unwrap();
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
            session: 50,
            core_root: BigUint::from_str(
                "6875536393007542918842449034124913087551795227982279536239765647106595144249",
            )
            .unwrap()
            .into(),
            pol_ledger_aged: BigUint::from_str(
                "14573153312699724670263660505956332253091318475916637993115041966847860184870",
            )
            .unwrap()
            .into(),
            pol_epoch_nonce: BigUint::from_str(
                "3805415232646885369110410132699703400676617784643552178853600292631771427767",
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
            index: 9,
        };
        let wallet_data = PoQWalletInputsData {
            slot: 2_266_263_054,
            note_value: 50,
            transaction_hash: BigUint::from_str(
                "12148629438803036174340995230158965196688969608035594138094620482702296371599",
            )
            .unwrap()
            .into(),
            output_number: 222,
            aged_path_and_selectors: [
                (
                    "18237978478669705950059435697605241574881071994070796386293905659474148880470",
                    true,
                ),
                (
                    "4501745438943720711017839691727104988688468076478939051701809724577989014515",
                    true,
                ),
                (
                    "11720761428331818936690098931068952571424518287435485250732856802185119611228",
                    true,
                ),
                (
                    "19655154067315493284547301261375636075325111723884605340262112215026354438646",
                    true,
                ),
                (
                    "11339363181286927957478687179353877264668727917846975095597428245661600052829",
                    true,
                ),
                (
                    "16785131141639872641883962758975023641630972205274801994795714858334304096444",
                    false,
                ),
                (
                    "13750879343626947092930102823112310926415677917372324244643199666453735657321",
                    true,
                ),
                (
                    "8895073096459472995222973210785572326524311560313045664713083909967531327261",
                    true,
                ),
                (
                    "11492649238607241231058872066020077038643276043635513423073046229246520601139",
                    false,
                ),
                (
                    "18810636223177667794907858473089745662995469239919346798622591227537682559617",
                    true,
                ),
                (
                    "18588785886490789118893509868987655521834104095990134045490758803714918034534",
                    true,
                ),
                (
                    "1641380842515409133443245709362717461855466677844607014467099244919420201433",
                    true,
                ),
                (
                    "16704985248744451048849636728643522064683733780773743297279926785463488554934",
                    true,
                ),
                (
                    "11575073455610773188661662745478748838472949444343802705273753409416989706683",
                    true,
                ),
                (
                    "14385607361730490972508094197802370695249366192582605202655579583048295233879",
                    true,
                ),
                (
                    "2884151011469266283891228693278155981508418781385084110140142066967418090601",
                    true,
                ),
                (
                    "6407397368940307560103534817654567176494656633551884568366374919982401149321",
                    true,
                ),
                (
                    "5374475538768795725988377434324447214846006511951873887323465775852418804279",
                    false,
                ),
                (
                    "3615253293369746535512838633482190880686722773785956818822342504305127958863",
                    true,
                ),
                (
                    "7564170871184826583775396105524119472532708495055268051276872554445227977682",
                    false,
                ),
                (
                    "9390938333991420386791718154791388602999575377599162160025226501688881848757",
                    false,
                ),
                (
                    "941725622156210378219513241150597534138029505249387126860602127375196976975",
                    false,
                ),
                (
                    "20507978105485327140069433067699930337355308411207695142066376163008319594636",
                    false,
                ),
                (
                    "10639517185230927462615451568792785679527330831645616474518713833542907912599",
                    false,
                ),
                (
                    "18693082727074653759586714920218257350214682854151032370036278655359134233533",
                    false,
                ),
                (
                    "6929087490588119833488647182526313581267865338253461315486502436087137247557",
                    false,
                ),
                (
                    "19720378834623438319396490617176291123353207639720146553481405267728215915646",
                    true,
                ),
                (
                    "10129616177078460633974828808926051341672720564004830541018518185279600298158",
                    true,
                ),
                (
                    "4926948498340260254267953024687810382855532705523877027354805364134602798376",
                    false,
                ),
                (
                    "11011911292325994602279138734916611177587703522203504406070605229796949227956",
                    true,
                ),
                (
                    "2158669270842819841244704278036309817315849583650319871797832060902064002879",
                    false,
                ),
                (
                    "12360484986632180120425683160096745749069522136415034895976396720242342287530",
                    false,
                ),
            ]
            .map(|(value, selector)| (BigUint::from_str(value).unwrap().into(), selector)),
            slot_secret: BigUint::from_str(
                "20927854186926112503794368719673767167025347308808917082125697791963986116825",
            )
            .unwrap()
            .into(),
            slot_secret_path: [
                "1225173802040324236405617745624838117484449529135043390430898518099064747853",
                "19585338137966528678196149551056651190231566642271975243708504262907110885622",
                "8348333310992479549304886377703158726267989658660768370715115349131807987114",
                "5948540813574035184412460853806475994826776598802383292601001109364896828466",
                "8140870220675253711199146045706639472434304671865126308553884953930510717621",
                "9262955421767582444829159819083215961654241740686018475846126567110197230917",
                "14951868467460707260553894605238029560450569114919442796919128352308849623649",
                "8510774729006509321556512715502389328103651664387900988205019708801757076366",
                "20900460075483793148068671279576650087261953937028017755245585017447512914750",
                "384915758890388362505432806061401251800031517824819185336257781646795138618",
                "10992519771064413899499191586449825247321858327467797762482625107489429264903",
                "21283168662723518457881915508699518865171751380903696821401070174514225128730",
                "10303785840964635525399342555752137543733203286266967851030023713672088691167",
                "20113367523296131578013360054645566596158242080212156222055451921882377734194",
                "13349578717287644955277799783193449479281114629532114665050542427557006705327",
                "16541846882211438606390961797300007158135676385585768525024901188577534190824",
                "13844796922529183115987049445127647507818329597931243910490487384948467860360",
                "20245876514022691863990025822121372771077188783979426686039528255964329329348",
                "11486629162449707619580256569540681475488364778411907959638018302609170995373",
                "5188798632185199307511276114334519908651512275141122287679451954721343825609",
                "8712426004201499578948791385972273690427855812880265912622209806523379636097",
                "8446974873971783220139208929932500487151277086116859350055177440181695875375",
                "4970006922262909831994162001232504704781770049385220920737843431118964193209",
                "15301154223135237656175576443388715926502848085817881265070939249939504624964",
                "12920801583415769499211228096854517079681922143077507690296954188666862723556",
            ]
            .map(|value| BigUint::from_str(value).unwrap().into()),
            starting_slot: 2_262_363_728,
        };

        let witness_inputs =
            PoQWitnessInputs::from_leader_data(chain_data, common_data, wallet_data).unwrap();
        let (proof, inputs) = prove(witness_inputs).unwrap();
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
