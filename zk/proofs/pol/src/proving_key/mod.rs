use std::{convert::Into, sync::LazyLock};

use crate::verification_key::PolVerifyingKey;

pub struct PolProvingKey(Box<[u8]>);

#[expect(clippy::large_include_file, reason = "Proving key is large")]
static POL_PROVING_KEY: LazyLock<PolProvingKey> =
    LazyLock::new(|| PolProvingKey(include_bytes!("pol.zkey").to_vec().into_boxed_slice()));
