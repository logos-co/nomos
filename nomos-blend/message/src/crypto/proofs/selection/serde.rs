pub(super) mod selection_randomness_serde {
    use groth16::Fr;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(_input: &Fr, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        todo!("Implement proper serialization later.")
    }

    pub fn deserialize<'de, D>(_deserializer: D) -> Result<Fr, D::Error>
    where
        D: Deserializer<'de>,
    {
        todo!("Implement proper serialization later.")
    }
}
