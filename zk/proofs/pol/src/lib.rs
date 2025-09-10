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

pub use crate::lottery::{P, compute_lottery_values};
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
            slot_number: 156,
            epoch_nonce: Fr::from(51654u64),
            total_stake: 5000,
            aged_root: BigUint::from_str(
                "13222315389447979533409058900399666127736845705057482510556088917353766377342",
            )
            .unwrap()
            .into(),
            latest_root: BigUint::from_str(
                "4309379669440222376041872524181651770509565787320394066165325089044228554447",
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
                "342547056393458855845753815217765206108386140749704829495176800472973718583",
            )
            .unwrap()
            .into(),
            output_number: 220,
            aged_path: [
                "355539461562315939847017089826648849258602931775631537275382558975221274311",
                "16826537986358661545184239639257626838931869526075304606659425925534195818026",
                "7645327622209725769822169909267710681135944633498641274473414629893178188424",
                "8102083501789230133333602933255855787637763236319603575676107899851821047270",
                "3080215515654370980849029407000388791615103821522326797949596790100825759888",
                "73235820408974732432501961577720842528787738305512597914313484009859815582",
                "1843253802689779184275474571762601383554538624967352335511789358949969913822",
                "4581892631567573432073351610864355565428572261676559555842413912811600166255",
                "3556438886726160277491739389153676639305288126419436605731651074246752174920",
                "4566701893519242132544993757822768248666048692474007434932777757676954680341",
                "20302047852885742090938291804797861256242100712432448491384247046064608750867",
                "3904931822969038294698060299768179891800851776053938973204786596569337701750",
                "17178443873246912408416954620990227704628422149339675490928782202142366809332",
                "1487572208381974717921290595573771600937758267539187363623454678743099666482",
                "17510006851769197611326263211882199202726302414816066010636712870911150116370",
                "14130231457745824847037994934895873146624450419739873786004920596920670953897",
                "7211197764341785921903425329343999752314536557012629972987183098273490521962",
                "9288828783945600844945476872935510676708637763230009859687432540412349348067",
                "15685437032282464853850624534302898987692624555961055606129365114087468546118",
                "3337705642581960681339692743984017356410225237772799566114672282878571236636",
                "11498569106879739166945903997971066883368864711452964428426191470464422656762",
                "3105574661137438711505991153180557964015933589623076720998377895499558878959",
                "9412098442098843175718124841115206748454429227589778686062175166148565620798",
                "6873628343148324695398561815484645884242089171709464245955633708263929703004",
                "9777655060078935654599917903194449746849937307416984713710455438842784530709",
                "12243488683588300574069355560424085981643745828209134494320578211080840648226",
                "3811659922529886577059227216447060096939173188901696115546843073686697787176",
                "9845604542147172776787838476913269174763479947754306596727885057322430153052",
                "13900714669723911465805989039673194962095124410503882813875857639786499235179",
                "4039351686791521202215103888696432750512078091643050507169196715353149306004",
                "18625655501467400066183652549133193946497610289401830450729987994777415560861",
                "8955342809739140049570676545707745141789039205457874222073524857549064252978",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            aged_selector: [
                "0", "1", "1", "0", "0", "0", "1", "0", "0", "0", "1", "0", "0", "0", "1", "1",
                "1", "0", "0", "0", "0", "1", "0", "1", "0", "0", "1", "1", "0", "0", "0", "0",
            ]
            .into_iter()
            .map(|s| match s {
                "1" => true,
                "0" => false,
                _ => panic!("Invalid value for aged_selector"),
            })
            .collect(),
            latest_path: [
                "18307412677817065895119782830854291051132770605956693383483450198506134415641",
                "4783161453680511404284865290700183362273225684089550280393811916313469767028",
                "5523553847453024668929952368002425388628155733620818124339673870739590821003",
                "12811147235494727954852501557698230844524359580459680156007599659837578960961",
                "13000615396878469046829304045597527164162274874937305391907948248153794072025",
                "10822575871808310073190776063316714641925095986283344709689823908327518196668",
                "6146772218159369993996058713208956040984898061166778119925433882117040854560",
                "12580491303497515641291553161069327032049712203465853416305723615611821239247",
                "13374941268187068896113130262855308871266992496729850214527184807215422205576",
                "10953657611216586004931945743563325354407535688596365752949850766942721677774",
                "3074931004978935271825113052868843223575886260294074313625829339039150765838",
                "15651657893241825071863524487094044702940986514455849330611761112969214427914",
                "8196936750212814120861289835890153212626766750860238629838240118072614482445",
                "15932608887600089656232192023480280720825742040321128410582886063221567522584",
                "10801416274816221412798459635656837717571852042226574136292546159039639895981",
                "5016750250215190204358799803499195205917748406364441462556003510301044912675",
                "1882012735167852219098996772405718370912475415150588794547458885113997319521",
                "21235528139646391794233841517835833844249846365423334386948609495654036034048",
                "21601105933006131432770604861135680952756737765980665588301638041304371203510",
                "8664857433313996645380214914573736334287188728233276292468005528527805572702",
                "15942600505145392614446229084462833735291791199886560511265625749890206768919",
                "1303870087267336487722999336964578964154661424518206693081604322572741753479",
                "20883486318774879819569102575283769900621793542345663714284600183506486012744",
                "2956684990143397519093779600615954132046686672275572483930828236854860559499",
                "1512972222437685797166033041235615997114607360526726794792523084481135399031",
                "5185833167476012487989915880528904543092297254012878351886885358393570985797",
                "21692068914999532921549958945055681363964947330201071946927641668524748507614",
                "6351226940807225596766320341213538694415074120895011464335908311323735716929",
                "14187716759775198356406559514726733815358050892107047224657546032495024973193",
                "14895712672032893207082782079691200815547341128780269634738931335957276131747",
                "1302762485726998729792542557724862406192971498757651704804633039204337295609",
                "16540107168413016819328899341030030465880644448977747817001859981787550393731",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            latest_selector: [
                "0", "0", "0", "0", "1", "1", "0", "1", "1", "0", "1", "1", "1", "0", "0", "0",
                "1", "0", "1", "0", "1", "1", "1", "1", "0", "0", "0", "1", "0", "1", "1", "0",
            ]
            .into_iter()
            .map(|s| match s {
                "1" => true,
                "0" => false,
                _ => panic!("Invalid value for aged_selector"),
            })
            .collect(),
            slot_secret: BigUint::from_str(
                "9606490998872466911909593909876492726341183063228076543834638639215685265768",
            )
            .unwrap()
            .into(),
            slot_secret_path: [
                "7888702907808771377482001443836242893554362050408304859800607373925805342306",
                "14311309925326878621486771461578841905101790067356797302714362607578665484597",
                "6789372107186012734178044954556631648299828724993164918839488464310490477043",
                "4519898308129565528279693059761640940281021517315938772669842767998735565950",
                "17655962792707070699795694294358856286803082593055097721358260206694569340070",
                "21489238192566139263320136698673112723107043224817512081250651825496540556045",
                "657047898091191980507411320098677028418852602926283106591005079523199166023",
                "12834808623398191627366452194275870550566814312315164790619115156862962177233",
                "7656803064580496447821858509676956973245493908752885929254884925898223363898",
                "11588844627202488542780189688705163320249282740624378815253320858540960938447",
                "1905073679320528345992352205008048488911824846436867160459164595199807236917",
                "13989990378441373738849958480424850035242976654881217701721060391042282010139",
                "16992268943519984228723247913403792399128907899215195775012828314557619843155",
                "12513321039276420940064932896706562626566981248901453936038789692753044498245",
                "6950908076663542712224469613279704490445462098841027638327451467172744603621",
                "4512690019799708921400800690385246055152976709605306296010119724514884954619",
                "18520654094397232048465156943775614448417651010909925364788296178005924024161",
                "10719913364841212039052330479823929839459933617078783028623305387502785627079",
                "17123177877951942394691893829463785394728768524873567435687601244639854941616",
                "11224154589452295910027604320401114667811725130719022791664102884851024249906",
                "14250091832044778930923121195904939602188740232183731446634376323131777942400",
                "19177346936333888813605934432581455899632768357750840434282456924220091202414",
                "16852499889426005813582407823289805225085092018763631877344336649081260907315",
                "2197457218783212233021050576100360111608276208774605907402941418425584249130",
                "219491350727612244658884277780845349189405786283479666243313543808487260557",
            ]
            .into_iter()
            .map(|value| BigUint::from_str(value).unwrap().into())
            .collect(),
            starting_slot: 16,
        };
        let witness_inputs = PolWitnessInputs::from_chain_and_wallet_data(chain_data, wallet_data);

        let (proof, inputs) = prove(&witness_inputs).unwrap();
        assert!(verify(&proof, &inputs).unwrap());
    }
}
