use kzgrs::{global_parameters_from_randomness, GlobalParameters};
use once_cell::sync::Lazy;

// Reexport global parameters loading from file.
pub use kzgrs::global_parameters_from_file;

pub static GLOBAL_PARAMETERS: Lazy<GlobalParameters> = Lazy::new(|| {
    println!("WARNING: Global parameters are randomly generated. Use for development only.");
    let mut rng = rand::thread_rng();
    global_parameters_from_randomness(&mut rng)
});

#[cfg(test)]
mod tests {
    use std::fs::File;

    use ark_serialize::{CanonicalSerialize, Write};
    use kzgrs::global_parameters_from_randomness;

    #[test]
    #[ignore = "for testing purposes only"]
    fn write_random_kzgrs_params_to_file() {
        let mut rng = rand::thread_rng();
        let params = global_parameters_from_randomness(&mut rng);

        let mut serialized_data = Vec::new();
        params.serialize_uncompressed(&mut serialized_data).unwrap();

        let mut file = File::create("./kzgrs_test_params").unwrap();
        file.write_all(&serialized_data).unwrap();
    }
}
