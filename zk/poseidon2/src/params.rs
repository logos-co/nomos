use ark_bn254::Fr;
use jf_poseidon2::{Poseidon2Params, define_poseidon2_params};

use crate::constants::{MAT_DIAG3_M_1, RC3_EXTERNAL, RC3_INTERNAL};

define_poseidon2_params!(
    Poseidon2Bn254Params,
    3,
    5,
    8,
    56,
    RC3_EXTERNAL,
    RC3_INTERNAL,
    MAT_DIAG3_M_1
);
