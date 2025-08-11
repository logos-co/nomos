use serde::Deserialize;

use crate::{curve::Curve, protocol::Protocol};

#[derive(Deserialize)]
pub struct ProofJsonDeser {
    pub protocol: Protocol,
    pub curve: Curve,
    pub pi_a: [String; 3],
    pub pi_b: [[String; 2]; 3],
    pub pi_c: [String; 3],
}

#[cfg(test)]
mod tests {
    use ark_bn254::Bn254;
    use ark_ec::pairing::Pairing;
    use ark_ff::{Fp, Fp2};
    use num_bigint::BigUint;
    use serde_json::json;

    #[test]
    fn stringified_g1() {
        use crate::proof1::serde_proof::StringifiedG1;

        let _: <Bn254 as Pairing>::G1Affine = StringifiedG1([
            "8296175608850998036255335084231000907125502603097068078993517773809496732066"
                .to_string(),
            "8263160927867860156491312948728748265016489542834411322655068343855704802368"
                .to_string(),
            "1".to_string(),
        ])
        .into();
    }

    #[test]
    fn stringified_g2() {
        use crate::proof1::serde_proof::StringifiedG2;
        let _: <Bn254 as Pairing>::G2Affine = StringifiedG2([]).into();
    }
    #[test]
    fn deserialize() {
        use crate::proof1::serde_proof::ProofJsonDeser;
        let data = json!(
                    {
          "pi_a": [
            "8296175608850998036255335084231000907125502603097068078993517773809496732066",
            "8263160927867860156491312948728748265016489542834411322655068343855704802368",
            "1"
          ],
          "pi_b": [
            [
              "21630590412244703770464699084160733144935501859194730009968664948222752546282",
              "2360176260887090528387414040841390178721803616623769558861196687249493928600"
            ],
            [
              "19520030071777612089051083418787870247443252641482678846010900794231980067541",
              "10365922284519340998921178202220836853052351283418810378278857066381010824566"
            ],
            [
              "1",
              "0"
            ]
          ],
          "pi_c": [
            "6696664968468451496397455124742234961189848064077552976860754045639269197981",
            "6523385944235793127051945618289282151393577593495757596060209123245519772531",
            "1"
          ],
          "protocol": "groth16",
          "curve": "bn128"
        });
        let proof: ProofJsonDeser = serde_json::from_value(data).unwrap();
    }
}
