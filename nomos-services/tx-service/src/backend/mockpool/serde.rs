use std::hash::Hash;

use linked_hash_map::LinkedHashMap;
use serde::{
    Deserialize, Deserializer as DeserializerTrait, Serialize, Serializer as SerializerTrait,
};

pub(super) fn serialize_pending_items<Key, Item, Serializer>(
    items: &LinkedHashMap<Key, Item>,
    serializer: Serializer,
) -> Result<Serializer::Ok, Serializer::Error>
where
    Key: Eq + Hash + Serialize,
    Item: Serialize,
    Serializer: SerializerTrait,
{
    items.iter().collect::<Vec<_>>().serialize(serializer)
}

pub(super) fn deserialize_pending_items<'de, Key, Item, Deserializer>(
    deserializer: Deserializer,
) -> Result<LinkedHashMap<Key, Item>, Deserializer::Error>
where
    Key: Deserialize<'de> + Eq + Hash,
    Item: Deserialize<'de>,
    Deserializer: DeserializerTrait<'de>,
{
    Vec::<(Key, Item)>::deserialize(deserializer)
        .map(|items| items.into_iter().collect::<LinkedHashMap<Key, Item>>())
}
