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
pub fn verify(proof: &PoQProof, public_inputs: &PoQVerifierInput) -> Result<bool, impl Error> {
    let inputs = public_inputs.to_inputs();
    groth16::groth16_verify(verification_key::POQ_VK.as_ref(), proof, &inputs)
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use num_bigint::BigUint;
    use groth16::{Field, Fr};
    use super::*;

    #[expect(clippy::too_many_lines, reason = "For the sake of the test let it be")]
    #[test]
    fn test_core_node_full_flow() {
        let blend_data = PoQBlendInputsData {
            core_sk: BigUint::from_str("7125861420969474621453722788282153604188736277629052106834372451719532284045").unwrap().into(),
            core_path: [
                "14878463682611476851448824807152743206520205891978019151885286571177999719952",
                "9388887435696504241693348585256264016789657424400656646076815081872523244993",
                "6561149924862232640331751300958465019453283187526343522150703758630055162202",
                "3291009421721582096615250931921100233097439292778906729009747655084461479359",
                "2783144780605061209017850214544546139188085554409235350816397619949110934578",
                "10624500957652693594415197778281859653583484085812749346674614354053796570383",
                "4905325074443688546145193261394026681624674672319321984043800569606306156444",
                "13913037263814338426127575777751914974934359538734406487868526368472803089804",
                "15917064814391945817955996049953152268867366761001083205117537576241104486152",
                "17227979983938017595061461817275061601853449522073357018832808006910734237125",
                "1208957932677550553616678529057798647479792564528759306385741900211694285136",
                "18943804249743947245629960221244858823975653440107777767008653284342437442150",
                "3361023263588933517014035643005745156880521016298650522530289194504240799784",
                "20461549240523476276686820244745411455542848810421365089660110212033012232813",
                "5294970547972824247464120523276987734323169570584974092767486518498547542526",
                "20682390013526987725484405281445142772266325239833268585843477419471428571246",
                "5061268412180156656773565635505967578526104467550145656542611607552171403633",
                "5945645214577050228115610577038723870448342023994521095337791192601540145034",
                "3562680615416927709364632473337550455676807698264428102435546544735661898337",
                "13011524506780490924075155635847245006692784529938157269996588544947824705256"
            ]
                .into_iter()
                .map(|value| BigUint::from_str(value).unwrap().into())
                .collect(),
            core_path_selectors: [
                "1",
                "0",
                "1",
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
                "1",
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
            session: 156,
            core_root: BigUint::from_str("2199781065250742392682930942351286226901861525504935003871751716970499371863").unwrap().into(),
            pol_ledger_aged: BigUint::from_str("3102442276359875253043635989104074235458844955222731305653757143681233508320").unwrap().into(),
            pol_epoch_nonce: BigUint::from_str("10910899942027430179301162417485809835361600109400454821538816576096522997064").unwrap().into(),
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
            common_data.try_into().unwrap(),
            blend_data.try_into().unwrap(),
        );

        let (proof, inputs) = prove(&witness_inputs).unwrap();
        assert!(verify(&proof, &inputs).unwrap());
    }

    #[expect(clippy::too_many_lines, reason = "For the sake of the test let it be")]
    #[test]
    fn test_leader_node_full_flow() {
        let chain_data = PoQChainInputsData {
            session: 156,
            core_root: BigUint::from_str("2199781065250742392682930942351286226901861525504935003871751716970499371863").unwrap().into(),
            pol_ledger_aged: BigUint::from_str("3102442276359875253043635989104074235458844955222731305653757143681233508320").unwrap().into(),
            pol_epoch_nonce: BigUint::from_str("10910899942027430179301162417485809835361600109400454821538816576096522997064").unwrap().into(),
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
        let wallet_data = PoQWalletInputsData {
            slot: 4174919025,
            note_value: 50,
            transaction_hash: BigUint::from_str("2227645356271445998894686812455136909605201127574909722436861780579834467651").unwrap().into(),
            output_number: 39,
            aged_path: [
                "5209120414289630220373030157042635185920402501572859819431565620891026888517",
                "14348330770748633300236382322534612672007971870102511642520446047120569029012",
                "18165887102959708220422111581089483037182872226059862439024371746554689469894",
                "21231915242568899046059799622012884613337424919702205355157261559605732662088",
                "4971246466351133517225333153715197952408849291343314926824983087865286686608",
                "13239919917052610493571376483006046582288008830748589296405391716481064335334",
                "12861367099317265345503563779101434981553930574397804719730394285017247121502",
                "11637255908192377316082556839126481654901578920421108666545686736973559701962",
                "18359622389327961884108424643648798957670828997267336654354498195527346230542",
                "6190259278749573906656103250809449419251125706995486584655433976110583443709",
                "18983117806475592208833335808652900760889721640777142231528225371756205516503",
                "16194636159942374205552157363079476845027359384411043828748313968235968616592",
                "18183591251641718311519169914078585885331713873781751604880569049547756158441",
                "6361522692180625032585893356551194993336118820380469264738744702626197730324",
                "16673419852276820183544844722143175740001213752715411020821126808167249262263",
                "10944117788053125132954635655941639247079610918745152471556550663533320063348",
                "17544574573839366516291437373254235586759131654889942700995023396834871787009",
                "17821465460068083900011575802983743668570953329104058478331266871915788373608",
                "16149432259630204465395555641455282482365126028322673454071193196908196851064",
                "8164635552912902656888976502909834109016813944431605190216004370629001943115",
                "5986907623863098440566650866001305898348199924726067984515620352362323834316",
                "2609101196607518800453367665572213422167071737408098110937854305301567772995",
                "10514830240714710632421081131262741009148988794608198534100910518454621480844",
                "5108286611423351811539356027988177857140204209458132077731811938499580004719",
                "14284833161557900835550384057761471735575660907051383659604205211410359338292",
                "16354330668137471851733800499265388194452644511884343136096255960554110989949",
                "8056859050802970478870689314408646333051295098586834627424745238465081981761",
                "5552517244454664309144218427832184767449522664167835058650303300377815890731",
                "15729657792244239365968011144823674099258611046524166709939016259242508702421",
                "6732369702260158157637008866020212163709297999687886687750343189916189201175",
                "130756458878880639164959420531352277267512600211344489049287791160488512335",
                "12770929944481820510904418458026842975093991519170442545003446997888041187756"
            ]
                .into_iter()
                .map(|value| BigUint::from_str(value).unwrap().into())
                .collect(),
            aged_selector: [
                "0",
                "0",
                "0",
                "1",
                "0",
                "1",
                "0",
                "0",
                "1",
                "1",
                "1",
                "1",
                "0",
                "1",
                "0",
                "1",
                "1",
                "1",
                "0",
                "0",
                "0",
                "0",
                "0",
                "0",
                "0",
                "0",
                "0",
                "0",
                "0",
                "1",
                "1",
                "0"
            ]
                .into_iter()
                .map(|s| match s {
                    "1" => true,
                    "0" => false,
                    _ => panic!("Invalid value for aged_selector"),
                })
                .collect(),
            slot_secret: BigUint::from_str("5722467179161421754206930997657131423925382580673198020472881498163450546660").unwrap().into(),
            slot_secret_path: [
                "20975664525172751767250489132860044606799629364931824559744506541810532750888",
                "10841758515489344076588401677412750175142993816121535455017299743892652944652",
                "5741522884765328289358128627651065997306115537605379014033216861835507667015",
                "18695636627511434517472276674031939547054255903506675775239141177052932040948",
                "6193365015195211654785029081050125192617587220236463588496927474497689242411",
                "20167363324078436232442817070300709506874743617535565415962605537084347212828",
                "15462183735081000301022863458941161545011062136120685149583450500588052535726",
                "3862317562403459901451072169970952007678634991212924038273499682804181971022",
                "21216373485739749331659327841633817700719203233116627359243139996319309422939",
                "5068171371483987402630277588680953371912488263219335006288924449696825826046",
                "21425963632973825027678860200561502572367544106229053936442569804091137063597",
                "12332756947859355432501995645406629175525233617694673088204260258949821704438",
                "6252195479167767998064731100519090928232593437466714896484044343228585342969",
                "6056682955590596789971549938432419715915709851967057261688605402090773506885",
                "19080205765775485092940297548631671113252787315250020722541700547857685659202",
                "1352249907503342842473577809172643194125720017149780580522319004952530682504",
                "581731937606998811896506673470971438204365871909337751002557628975059938202",
                "654837592853290910447872164850246483401287238696056596223968372620480085344",
                "21754850508901076247343227035663454881234883886180478558247555277865737769968",
                "18144690474496282375396893295288609853809609525939285284065654537432135037795",
                "20941432967124009050808470967080487881581107723690412899791848792012898818077",
                "18758180416311326857434669660805588410270716566247280586656615961188569426708",
                "6750620074259402069591964342886834664661124155350208623339173835760128003933",
                "13366497895290652259514176021544854004993099191775108406167913821606537445363",
                "10413941207358332083823433195273755271649539846824244106965474417194047162677"
            ]
                .into_iter()
                .map(|value| BigUint::from_str(value).unwrap().into())
                .collect(),
            starting_slot: 4144152255,
        };

        let witness_inputs = PoQWitnessInputs::from_leader_data(
            chain_data.try_into().unwrap(),
            common_data.try_into().unwrap(),
            wallet_data.try_into().unwrap(),
        );

        let (proof, inputs) = prove(&witness_inputs).unwrap();
        assert!(verify(&proof, &inputs).unwrap());
    }
}
