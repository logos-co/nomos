use std::str::FromStr;

use ark_bn254::{Fq, Fq2, G1Affine, G2Affine};
pub struct StringifiedG1(pub [String; 3]);

impl TryFrom<StringifiedG1> for G1Affine {
    type Error = <Fq as FromStr>::Err;
    fn try_from(value: StringifiedG1) -> Result<Self, Self::Error> {
        let [x, y, _] = value.0;
        let x = Fq::from_str(&x)?;
        let y = Fq::from_str(&y)?;
        Ok(Self::new_unchecked(x, y))
    }
}

pub struct StringifiedG2(pub [[String; 2]; 3]);

impl TryFrom<StringifiedG2> for G2Affine {
    type Error = <Fq as FromStr>::Err;

    fn try_from(value: StringifiedG2) -> Result<Self, Self::Error> {
        let [[x1, y1], [x2, y2], _] = value.0;
        let x = Fq2::new(Fq::from_str(&x1)?, Fq::from_str(&y1)?);
        let y = Fq2::new(Fq::from_str(&x2)?, Fq::from_str(&y2)?);
        Ok(Self::new(x, y))
    }
}
