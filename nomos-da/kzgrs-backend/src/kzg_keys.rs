use std::sync::LazyLock;

// Reexport global parameters loading from file.
pub use kzgrs::{
    ProvingKey, VerificationKey, proving_key_from_file, proving_key_from_randomness,
    verification_key_proving_key,
};

pub static PROVING_KEY: LazyLock<ProvingKey> = LazyLock::new(|| {
    println!("WARNING: Proving key is randomly generated. Use for development only.");
    let mut rng = rand::thread_rng();
    proving_key_from_randomness(&mut rng)
});

pub static VERIFICATION_KEY: LazyLock<VerificationKey> =
    LazyLock::new(|| verification_key_proving_key(&PROVING_KEY));

#[cfg(test)]
mod tests {
    use std::fs::File;

    use ark_serialize::{CanonicalSerialize as _, Write as _};
    use kzgrs::proving_key_from_randomness;

    #[test]
    #[ignore = "for testing purposes only"]
    fn write_random_kzgrs_params_to_file() {
        let mut rng = rand::thread_rng();
        let proving_key = proving_key_from_randomness(&mut rng);

        let mut serialized_data = Vec::new();
        proving_key
            .serialize_uncompressed(&mut serialized_data)
            .unwrap();

        let mut file = File::create("./kzgrs_test_params").unwrap();
        file.write_all(&serialized_data).unwrap();
    }
}
