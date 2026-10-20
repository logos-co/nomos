use lb_binary_codec::canonical::codec_fixtures;
use num_bigint::BigUint;

use crate::mantle::notes::{NoteCm, NoteNf};

codec_fixtures!(NoteCm, Self(BigUint::from(123u64).into()) => "7b00000000000000000000000000000000000000000000000000000000000000");
codec_fixtures!(NoteNf, Self(BigUint::from(123u64).into()) => "7b00000000000000000000000000000000000000000000000000000000000000");
