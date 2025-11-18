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
                "5045274541783442659561151068972413902748220855765236358944101275506044314784",
            )
            .unwrap()
            .into(),
            core_path_and_selectors: [
                (
                    "13635043105988394370806479734641820578803480350461643876934989638883759759849",
                    true,
                ),
                (
                    "15569824705262716322503953135511464478446677249131121424011752232730109894957",
                    true,
                ),
                (
                    "9889916710679987479576998107417140185885468277790796353020417083645208382484",
                    false,
                ),
                (
                    "14377154707653977213266940960784202674370614187890804450378832859854470392062",
                    false,
                ),
                (
                    "11702880399764456242217907209228343046933793663159241548420337994934577734290",
                    true,
                ),
                (
                    "8641620104850996280312298227589418284729143964231705191266308984600686855039",
                    true,
                ),
                (
                    "12548809280589979884639818482255622226797196025043311064482057635844661612489",
                    false,
                ),
                (
                    "6860424061918188642856357918025238090466127846913221514093548553896417877658",
                    true,
                ),
                (
                    "15667382230054901510984007897830811361616686776086554531899428200872707708789",
                    false,
                ),
                (
                    "2805204195850409877829281562122810978191546790891714902013108338201255803144",
                    true,
                ),
                (
                    "14176012542315849581807926212217731766171052887685695678110624362805401497133",
                    false,
                ),
                (
                    "9026395288722668232224009991896790362536234382540391700178060513567454756240",
                    true,
                ),
                (
                    "3156700195015880395416699984288140806873477201849992671785726413076532091348",
                    true,
                ),
                (
                    "14104067784489257540689408685241976507655974890724969598819514122600758135861",
                    false,
                ),
                (
                    "11535442141906213354342975770846766385357464641681517492681563282137042993148",
                    true,
                ),
                (
                    "21868241737906779629626831660804444147465300009916217592133385290254032076701",
                    false,
                ),
                (
                    "11923280012372605040210397252240956993169990740542499913602079047648851331201",
                    false,
                ),
                (
                    "8701266069831122841022077981608320159519028888585970225994836309543849934082",
                    false,
                ),
                (
                    "4879183687977904771599963280630267706164708980422492256026309138563035760541",
                    false,
                ),
                (
                    "19719268410722104491913524874506237675898769193247288026429684900723579314821",
                    true,
                ),
            ]
            .map(|(value, selector)| (BigUint::from_str(value).unwrap().into(), selector)),
        };
        let chain_data = PoQChainInputsData {
            session: 150,
            core_root: BigUint::from_str(
                "21292049223746198489791451629615207124585037184810647704343680422015934117688",
            )
            .unwrap()
            .into(),
            pol_ledger_aged: BigUint::from_str(
                "5258897564204371484754651327289975698729164844559240514816278414835515847992",
            )
            .unwrap()
            .into(),
            pol_epoch_nonce: BigUint::from_str(
                "13074834417645654048388749982665432291190432130956123025028946037849318319097",
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
            index: 3,
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
            session: 150,
            core_root: BigUint::from_str(
                "21168693355624964081262962917192243439399584016327547711374431696925535965199",
            )
            .unwrap()
            .into(),
            pol_ledger_aged: BigUint::from_str(
                "19823719763929955023351235531671775052081595577268897983308945385815868834685",
            )
            .unwrap()
            .into(),
            pol_epoch_nonce: BigUint::from_str(
                "3144134360916285681200330327498733555123961796254055320535184042551296494251",
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
            index: 5,
        };
        let wallet_data = PoQWalletInputsData {
            slot: 3_289_467_226,
            note_value: 50,
            transaction_hash: BigUint::from_str(
                "1588335181409502888774746310266236570741472330306475226466640509717078598706",
            )
            .unwrap()
            .into(),
            output_number: 5421,
            aged_path_and_selectors: [
                (
                    "16860439484379307711108930641900160650823217413221487136523439864983709949568",
                    false,
                ),
                (
                    "18512620113459060012248429503828347864824092863099023301249733700951062600218",
                    false,
                ),
                (
                    "3402365513763569232557139685875124709543595979261389905448919383495571483833",
                    false,
                ),
                (
                    "3200997231331868784368222741448145958815994602779076485220218404419745650483",
                    false,
                ),
                (
                    "21886766085405568811440605516052104549315797081433941614916735201722089731636",
                    true,
                ),
                (
                    "18259909044046566268349043585846410922930883024962819331410878139152116426855",
                    true,
                ),
                (
                    "150350313483881887235642230226324357426424001126377376836972925313420334465",
                    false,
                ),
                (
                    "2144910462541832147233967070681342596926930764520841281630351893113808477660",
                    true,
                ),
                (
                    "8591543243684162103814745950469682706675708328382696778203381630353974745058",
                    false,
                ),
                (
                    "4042712688180740936479464111985152077780885223365012664178634659496885710601",
                    true,
                ),
                (
                    "18074788836020793500412684928694624542371463425724540877998529932789261445737",
                    true,
                ),
                (
                    "9899301132757530860555096852116154870807916415644874895405540514261012983438",
                    false,
                ),
                (
                    "19744654466773604507014444972297409554609005342821669670514137494378372669456",
                    true,
                ),
                (
                    "10349382380032159206947325188398604688102057607360127306219013458558446106656",
                    false,
                ),
                (
                    "10448132109271091294662481798477947426143701166772078839949536960565500123323",
                    true,
                ),
                (
                    "21136180834774407486276855520082744438855544434797872047274015076676061101623",
                    false,
                ),
                (
                    "8867325454367339927280012276295662414614585956237448723996644995499782587149",
                    true,
                ),
                (
                    "9473589405160039802455814711923537345502519467058516800832657861853727805127",
                    false,
                ),
                (
                    "10994639035986231603617766832222814749822210358676296681368798311888325746526",
                    true,
                ),
                (
                    "6506878573251668310601616831709453186789981594486973287124937001342016524061",
                    true,
                ),
                (
                    "20508970057675942040030704645046647251147526584417348353233440232194565304841",
                    true,
                ),
                (
                    "21674719562410836885080281011834165434742016894257379192141458063787842942333",
                    true,
                ),
                (
                    "4237068400408705444482195927967653692458583930178086064622886043437859489958",
                    false,
                ),
                (
                    "19194540804228419890883630867555331280281538265379615415889919139179027764952",
                    true,
                ),
                (
                    "8898559918066330058740029908718432321439089486069607140509995519772426331062",
                    false,
                ),
                (
                    "19192734947877986437022106332867494653297516483789328805762742794616014699153",
                    false,
                ),
                (
                    "6484223705342079547721033531524581300907977157148516689675407709631483057609",
                    true,
                ),
                (
                    "10166935198807691434154060925567609305447646817273987016420881008802253247105",
                    true,
                ),
                (
                    "7659041861304660176585303577362779341451939161431894663838382864351388953632",
                    true,
                ),
                (
                    "18543150555437778484119488731389327468989095807947864016432603854448958657944",
                    false,
                ),
                (
                    "4648119664420510485521658874246544299375591790650741912115973912354893922155",
                    true,
                ),
                (
                    "9387574292302302086688238997653783767634549103623297050285710513530415658965",
                    true,
                ),
            ]
            .map(|(value, selector)| (BigUint::from_str(value).unwrap().into(), selector)),
            slot_secret: BigUint::from_str(
                "16235640693755774591075545924287887351872778106019397089174781154147809544641",
            )
            .unwrap()
            .into(),
            slot_secret_path: [
                "9654606962003773166844110348967069744511622835002709960358297318450411496520",
                "270906710413154341795392827657834559209933822487438378321914769678745395792",
                "11606438163535696378116826995928755918958353677328966087487517927850803538857",
                "654285987273811067993314790003663335498971747034377946628394465336460630334",
                "14993216274195038600568938918045832150843310335621610637231532525800646887714",
                "18690540049917953487646902277238466508505580766172378598677257221000337107972",
                "20062063298944718170328043084648762760713017977223901179380896667209979984154",
                "14623289252240870507821777659223842545888430292194052420540590786060791931269",
                "5339080893173260151573825575811102449308551526005217763255068009885050098192",
                "3867992670555790614785620821123114451675366317987939061522649677943813866264",
                "20260735181174361103071519372903634171048419505473148496563296463567271740948",
                "13963177443110386304132923052081897481532700907673598510444144472170699170435",
                "11278007904012859904334392434757548372774604475159582940403632490529267568981",
                "16140502668027858997496455951100522333729351018536273286683916218184477014438",
                "8985917670249720202383311776409446288167205623694539906392533443484067030687",
                "6255155043113241956063250778191676872986000457393277963567847388791770971226",
                "14943820625271670361122354312425544201066701197831792744982437854564814795627",
                "5490605383533069135649624247730540797722033064283681742824370568779372338296",
                "14175029071312126667620905782883053905514891371350073635899884591599848194977",
                "5228784118128876380945998987809154071993912369630032518705326083085572694248",
                "12786004136695109420223634097110294889179531034578608475999386088756144176788",
                "21354571735578042901187888523391076054601974830666214324264489740118224701381",
                "10095687086171813686640259742128326926815930958827860831327297426828483602674",
                "9209987843259184396245325258302729976030314240199724817206280946637218803871",
                "6868310072497609319663226576787110101064161477858850025126190061332281801352",
            ]
            .map(|value| BigUint::from_str(value).unwrap().into()),
            starting_slot: 3_287_992_768,
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
