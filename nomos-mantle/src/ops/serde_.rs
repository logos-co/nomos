use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub(crate) struct WireOp<const CODE: u8, Op> {
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
struct WireOpDes<Inner> {
    #[allow(dead_code)]
    pub opcode: u8,
    pub payload: Inner,
}

impl<const CODE: u8, Inner: Serialize> Serialize for WireOp<CODE, Inner>
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
        let op = WireOpDes::<Inner>::deserialize(deserializer)?.payload;
        Ok(WireOp { op })
    }
}
