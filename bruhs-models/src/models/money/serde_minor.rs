//! Persist exact amounts as decimal strings, accepting legacy integer records.
use serde::{Deserialize, Deserializer, Serializer};

#[derive(Deserialize)]
#[serde(untagged)]
enum Minor {
    Text(String),
    Signed(i64),
    Unsigned(u64),
}

impl Minor {
    fn integer<E: serde::de::Error>(self) -> Result<i128, E> {
        match self {
            Self::Text(value) => value.parse().map_err(E::custom),
            Self::Signed(value) => Ok(i128::from(value)),
            Self::Unsigned(value) => Ok(i128::from(value)),
        }
    }
}

macro_rules! minor_serde {
    ($name:ident, $number:ty) => {
        pub(crate) mod $name {
            use super::*;
            pub fn serialize<S: Serializer>(
                value: &$number,
                serializer: S,
            ) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&value.to_string())
            }
            pub fn deserialize<'de, D: Deserializer<'de>>(
                deserializer: D,
            ) -> Result<$number, D::Error> {
                let value = Minor::deserialize(deserializer)?.integer::<D::Error>()?;
                <$number>::try_from(value).map_err(serde::de::Error::custom)
            }
        }
    };
}
minor_serde!(signed, i64);
minor_serde!(unsigned, u64);

pub(crate) mod optional_signed {
    use super::*;
    pub fn serialize<S: Serializer>(value: &Option<i64>, serializer: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(value) => serializer.serialize_some(&value.to_string()),
            None => serializer.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<i64>, D::Error> {
        Option::<Minor>::deserialize(deserializer)?
            .map(|value| {
                i64::try_from(value.integer::<D::Error>()?).map_err(serde::de::Error::custom)
            })
            .transpose()
    }
}
