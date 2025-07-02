use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub struct WireOp<const CODE: u8, Op> {
    pub op: Op,
}

#[derive(Serialize)]
struct WireOpSer<'a, Inner>
where
    &'a Inner: Serialize,
{
    pub opcode: u8,
    pub payload: &'a Inner,
}

#[derive(Deserialize)]
struct WireOpDe<Inner> {
    // #[expect(dead_code, reason = "Op codes are just used for deserialization")]
    pub opcode: u8,
    pub payload: Inner,
}

impl<const CODE: u8, Inner> Serialize for WireOp<CODE, Inner>
where
    Inner: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        WireOpSer {
            opcode: CODE,
            payload: &self.op,
        }
        .serialize(serializer)
    }
}

impl<'de, const CODE: u8, Inner: Deserialize<'de>> Deserialize<'de> for WireOp<CODE, Inner> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let op = WireOpDe::<Inner>::deserialize(deserializer)?.payload;
        Ok(Self { op })
    }
}

pub fn serialize_op_variant<const CODE: u8, Op: Serialize, S: Serializer>(
    op: &Op,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    WireOpSer {
        opcode: CODE,
        payload: op,
    }
    .serialize(serializer)
}

pub fn deserialize_op_variant<'de, const CODE: u8, Op: Deserialize<'de>, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Op, D::Error> {
    let op = WireOpDe::<Op>::deserialize(deserializer)?;
    if op.opcode != CODE {
        return Err(serde::de::Error::custom(format!(
            "Invalid opcode {} for type {}",
            op.opcode,
            std::any::type_name::<Op>()
        )));
    }
    Ok(op.payload)
}
