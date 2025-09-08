use std::{
    ops::{Deref as _, Div as _},
    str::FromStr as _,
    sync::LazyLock,
};

use num_bigint::BigUint;
use num_traits::CheckedSub as _;

#[expect(clippy::too_long_first_doc_paragraph, reason = "we need it")]
/// :warning: There may be dragons
/// The [following constants](https://www.notion.so/nomos-tech/Proof-of-Leadership-Specification-21c261aa09df819ba5b6d95d0fe3066d?source=copy_link#256261aa09df800fbc88e5aae5ea7e06)
/// Use the str representation instead of the hex representation from the
/// specification because of types signed/unsigned representations. Constants
/// where computed in python which uses a signed representation of the integer.
/// `BigInt` type (signed), parsed the hex values correctly. But `BigUint` does
/// not for the motives just described. Using the `str` representation allows
/// use to choose the unsigned type.
pub static P: LazyLock<BigUint> = LazyLock::new(|| {
    BigUint::from_str(
        "21888242871839275222246405745257275088548364400416034343698204186575808495617",
    )
    .expect("P constant should parse")
});

/// :warning: There may be dragons
/// The [following constants](https://www.notion.so/nomos-tech/Proof-of-Leadership-Specification-21c261aa09df819ba5b6d95d0fe3066d?source=copy_link#256261aa09df800fbc88e5aae5ea7e06)
/// Use the str representation instead of the hex representation from the
/// specification because of types signed/unsigned representations. Constants
/// where computed in python which uses a signed representation of the integer.
/// `BigInt` type (signed), parsed the hex values correctly. But `BigUint` does
/// not for the motives just described. Using the `str` representation allows
/// use to choose the unsigned type.
static T0_CONSTANT: LazyLock<BigUint> = LazyLock::new(|| {
    // t0 constant :
    // 0x27b6fe27507ca57ca369280400c79b5d2f58ff94d87cb0fbfc8294eb69eb1ea
    BigUint::from_str(
        "1122720085251457488657939587576977282954863756865979276605118041105190793706",
    )
    .expect("Constant should parse")
});

/// :warning: There may be dragons
/// The [following constants](https://www.notion.so/nomos-tech/Proof-of-Leadership-Specification-21c261aa09df819ba5b6d95d0fe3066d?source=copy_link#256261aa09df800fbc88e5aae5ea7e06)
/// Use the str representation instead of the hex representation from the
/// specification because of types signed/unsigned representations. Constants
/// where computed in python which uses a signed representation of the integer.
/// `BigInt` type (signed), parsed the hex values correctly. But `BigUint` does
/// not for the motives just described. Using the `str` representation allows
/// use to choose the unsigned type.
static T1_CONSTANT: LazyLock<BigUint> = LazyLock::new(|| {
    // t1 constant:
    // -0x104bfd09ebdd0a57772289d0973489b62662a4dc6f09da8b4af3c5cfb1dcdd
    BigUint::from_str("28794005923809446652337194229268641024881242442862297438215833784455126237")
        .expect("Constant should parse")
});

pub fn compute_lottery_values(total_stake: u64) -> (BigUint, BigUint) {
    let total_stake = BigUint::from(total_stake);

    let lottery_0 = T0_CONSTANT.deref().div(total_stake.clone());
    let lottery_1 = P
        .checked_sub(&T1_CONSTANT.deref().div(total_stake.pow(2)))
        .expect("(T1 / (S^2)) must be less than P");
    (lottery_0, lottery_1)
}
