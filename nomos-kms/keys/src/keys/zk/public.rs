#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(transparent)]
pub struct PublicKey(#[serde(with = "serde_fr")] Fr);

impl PublicKey {
    #[must_use]
    pub const fn zero() -> Self {
        Self(Fr::ZERO)
    }

    #[must_use]
    pub const fn new(key: Fr) -> Self {
        Self(key)
    }

    #[must_use]
    pub const fn as_fr(&self) -> &Fr {
        &self.0
    }

    #[must_use]
    pub const fn into_inner(self) -> Fr {
        self.0
    }

    #[must_use]
    pub fn verify(&self, data: &Fr, signature: &Signature) -> bool {
        let mut pks = [const { Self::zero() }; 32];
        pks[0] = *self;
        Self::verify_multi(&pks, data, signature)
    }

    #[must_use]
    pub fn verify_multi(pks: &[Self], data: &Fr, signature: &Signature) -> bool {
        let inputs = match ZkSignVerifierInputs::try_from_pks((*data).into(), pks) {
            Ok(inputs) => inputs,
            Err(e) => {
                error!("Error building verifier inputs: {e:?}");
                return false;
            }
        };

        crate::verify(signature.as_proof(), &inputs).unwrap_or_else(|e| {
            error!("Error verifying signature: {e:?}");
            false
        })
    }
}

impl From<SecretKey> for PublicKey {
    fn from(secret: SecretKey) -> Self {
        secret.to_public_key()
    }
}

impl From<Fr> for PublicKey {
    fn from(key: Fr) -> Self {
        Self::new(key)
    }
}

impl From<BigUint> for PublicKey {
    fn from(value: BigUint) -> Self {
        Self(value.into())
    }
}

impl From<PublicKey> for Fr {
    fn from(public: PublicKey) -> Self {
        public.0
    }
}
