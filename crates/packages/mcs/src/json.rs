//! Strict JSON: a document with a duplicate object key is refused, as MCS
//! refuses it, before it is read as a value.

use std::collections::BTreeSet;
use std::fmt;

use serde::de::{self, DeserializeOwned, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

/// Walks a document and fails on the first duplicate key.
struct NoDuplicates;

impl<'de> de::Deserialize<'de> for NoDuplicates {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(NoDuplicatesVisitor)
    }
}

struct NoDuplicatesVisitor;

impl<'de> Visitor<'de> for NoDuplicatesVisitor {
    type Value = NoDuplicates;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, _: bool) -> Result<NoDuplicates, E> {
        Ok(NoDuplicates)
    }
    fn visit_i64<E>(self, _: i64) -> Result<NoDuplicates, E> {
        Ok(NoDuplicates)
    }
    fn visit_u64<E>(self, _: u64) -> Result<NoDuplicates, E> {
        Ok(NoDuplicates)
    }
    fn visit_f64<E>(self, _: f64) -> Result<NoDuplicates, E> {
        Ok(NoDuplicates)
    }
    fn visit_str<E>(self, _: &str) -> Result<NoDuplicates, E> {
        Ok(NoDuplicates)
    }
    fn visit_unit<E>(self) -> Result<NoDuplicates, E> {
        Ok(NoDuplicates)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<NoDuplicates, A::Error> {
        while seq.next_element::<NoDuplicates>()?.is_some() {}
        Ok(NoDuplicates)
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<NoDuplicates, A::Error> {
        let mut keys = BTreeSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !keys.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate JSON key {key:?}")));
            }
            map.next_value::<NoDuplicates>()?;
        }
        Ok(NoDuplicates)
    }
}

/// `data` as a value of `T`, refusing a duplicate key anywhere in it.
pub(crate) fn strict<T: DeserializeOwned>(data: &[u8]) -> Result<T, String> {
    serde_json::from_slice::<NoDuplicates>(data).map_err(|error| error.to_string())?;
    serde_json::from_slice(data).map_err(|error| error.to_string())
}

/// `data` as a JSON value, refusing a duplicate key anywhere in it.
pub(crate) fn strict_value(data: &[u8]) -> Result<Value, String> {
    strict(data)
}

/// The keys of the object `field` of the object in `data`, in the order
/// written: a parsed [`Value`] sorts them.
pub(crate) fn key_order(data: &[u8], field: &'static str) -> Result<Vec<String>, String> {
    struct Keys(Vec<String>);

    impl<'de> de::Deserialize<'de> for Keys {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_map(KeysVisitor)
        }
    }

    struct KeysVisitor;

    impl<'de> Visitor<'de> for KeysVisitor {
        type Value = Keys;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an object")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Keys, A::Error> {
            let mut keys = Vec::new();
            while let Some(key) = map.next_key::<String>()? {
                map.next_value::<de::IgnoredAny>()?;
                keys.push(key);
            }
            Ok(Keys(keys))
        }
    }

    struct Field(&'static str);

    impl<'de> Visitor<'de> for Field {
        type Value = Option<Vec<String>>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("an object")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut found = None;
            while let Some(key) = map.next_key::<String>()? {
                if key == self.0 {
                    found = Some(map.next_value::<Keys>()?.0);
                } else {
                    map.next_value::<de::IgnoredAny>()?;
                }
            }
            Ok(found)
        }
    }

    let mut deserializer = serde_json::Deserializer::from_slice(data);
    deserializer
        .deserialize_map(Field(field))
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("no object {field:?}"))
}
