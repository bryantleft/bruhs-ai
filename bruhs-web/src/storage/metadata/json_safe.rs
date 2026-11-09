//! Check JSON compatibility without materializing a second metadata tree.

use serde::ser::{self, Serialize};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy)]
pub(super) struct JsonSafe;

#[derive(Debug)]
pub(super) struct Unsupported;

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("value requires the structured JavaScript codec")
    }
}
impl std::error::Error for Unsupported {}
impl ser::Error for Unsupported {
    fn custom<T: std::fmt::Display>(_: T) -> Self {
        Self
    }
}

type Result = std::result::Result<(), Unsupported>;

macro_rules! scalar {
    ($($method:ident($ty:ty)),* $(,)?) => {$(
        fn $method(self, _: $ty) -> Result { Ok(()) }
    )*};
}

impl ser::Serializer for JsonSafe {
    type Ok = ();
    type Error = Unsupported;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    scalar! {
        serialize_bool(bool), serialize_i8(i8), serialize_i16(i16),
        serialize_i32(i32), serialize_u8(u8), serialize_u16(u16),
        serialize_u32(u32), serialize_char(char), serialize_str(&str),
        serialize_bytes(&[u8]),
    }
    fn serialize_i64(self, value: i64) -> Result {
        self.serialize_u64(value.unsigned_abs())
    }
    fn serialize_u64(self, value: u64) -> Result {
        if value <= MAX_SAFE_INTEGER {
            Ok(())
        } else {
            Err(Unsupported)
        }
    }
    fn serialize_i128(self, _: i128) -> Result {
        Err(Unsupported)
    }
    fn serialize_u128(self, _: u128) -> Result {
        Err(Unsupported)
    }
    // JSON's shortest f32 text differs from a JS Number promoted from f32.
    fn serialize_f32(self, _: f32) -> Result {
        Err(Unsupported)
    }
    fn serialize_f64(self, value: f64) -> Result {
        if value.is_finite() {
            Ok(())
        } else {
            Err(Unsupported)
        }
    }
    fn serialize_none(self) -> Result {
        Ok(())
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result {
        Ok(())
    }
    fn serialize_unit_struct(self, _: &'static str) -> Result {
        Ok(())
    }
    fn serialize_unit_variant(self, _: &'static str, _: u32, _: &'static str) -> Result {
        Ok(())
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(self, _: &'static str, value: &T) -> Result {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        value: &T,
    ) -> Result {
        value.serialize(self)
    }
    fn serialize_seq(self, _: Option<usize>) -> std::result::Result<Self, Unsupported> {
        Ok(self)
    }
    fn serialize_tuple(self, _: usize) -> std::result::Result<Self, Unsupported> {
        Ok(self)
    }
    fn serialize_tuple_struct(
        self,
        _: &'static str,
        _: usize,
    ) -> std::result::Result<Self, Unsupported> {
        Ok(self)
    }
    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> std::result::Result<Self, Unsupported> {
        Ok(self)
    }
    fn serialize_map(self, _: Option<usize>) -> std::result::Result<Self, Unsupported> {
        Ok(self)
    }
    fn serialize_struct(self, _: &'static str, _: usize) -> std::result::Result<Self, Unsupported> {
        Ok(self)
    }
    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        _: &'static str,
        _: usize,
    ) -> std::result::Result<Self, Unsupported> {
        Ok(self)
    }
}

macro_rules! sequence {
    ($trait:ident, $method:ident) => {
        impl ser::$trait for JsonSafe {
            type Ok = ();
            type Error = Unsupported;
            fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> Result {
                value.serialize(*self)
            }
            fn end(self) -> Result {
                Ok(())
            }
        }
    };
}
sequence!(SerializeSeq, serialize_element);
sequence!(SerializeTuple, serialize_element);
sequence!(SerializeTupleStruct, serialize_field);
sequence!(SerializeTupleVariant, serialize_field);

macro_rules! record {
    ($trait:ident) => {
        impl ser::$trait for JsonSafe {
            type Ok = ();
            type Error = Unsupported;
            fn serialize_field<T: ?Sized + Serialize>(
                &mut self,
                _: &'static str,
                value: &T,
            ) -> Result {
                value.serialize(*self)
            }
            fn end(self) -> Result {
                Ok(())
            }
        }
    };
}
record!(SerializeStruct);
record!(SerializeStructVariant);

impl ser::SerializeMap for JsonSafe {
    type Ok = ();
    type Error = Unsupported;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result {
        // The existing JSON-compatible JS serializer rejects non-string map
        // keys, even though serde_json would stringify integer keys for us.
        match serde_json::to_value(key) {
            Ok(serde_json::Value::String(_)) => Ok(()),
            _ => Err(Unsupported),
        }
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result {
        value.serialize(*self)
    }
    fn end(self) -> Result {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
