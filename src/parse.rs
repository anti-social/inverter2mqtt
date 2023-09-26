use std::fmt::Display;
use std::num::{ParseFloatError, ParseIntError};
use std::str::SplitAsciiWhitespace;

use serde::Deserialize;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};

use snafu::Snafu;
use snafu::prelude::*;

#[derive(Debug, PartialEq, Snafu)]
pub enum DeError {
    #[snafu(display("Custom error: {msg}"))]
    Message { msg: String },

    #[snafu(display("Unsupported type: {typ}"))]
    UnsupportedType { typ: &'static str },

    #[snafu(display("Missing field"))]
    MissingField,

    #[snafu(display("Expected value for field: {field}"))]
    ExpectedValue { field: &'static str },

    #[snafu(display("Expected single character value for field: {field}"))]
    ExpectedChar { field: &'static str },

    #[snafu(display("Expected float value for field {field}: {source}"))]
    ExpectedFloat { field: &'static str, source: ParseFloatError },

    #[snafu(display("Expected integer value for field {field}: {source}"))]
    ExpectedInteger { field: &'static str, source: ParseIntError },
}

impl DeError {
    fn unsupported_type(typ: &'static str) -> Self {
        Self::UnsupportedType { typ }
    }
}

impl de::Error for DeError {
    fn custom<T: Display>(msg: T) -> Self {
        DeError::Message { msg: msg.to_string() }
    }
}

/// Deserializer for space separated values
pub struct Deserializer<'de> {
    values: SplitAsciiWhitespace<'de>,
    field: Option<&'static str>,
}

impl<'de> Deserializer<'de> {
    fn from_str(input: &'de str) -> Self {
        Deserializer {
            values: input.split_ascii_whitespace(),
            field: None,
        }
    }

    fn field(&self) -> Result<&'static str, DeError> {
        self.field
            .ok_or_else(|| DeError::MissingField)
    }

    fn field_or_unknown(&self) -> &'static str {
        self.field.unwrap_or("<unknown>")
    }

    fn take_field(&mut self) -> Result<&'static str, DeError> {
        self.field.take()
            .ok_or_else(|| DeError::MissingField)
    }

    fn value(&mut self) -> Result<&'de str, DeError> {
        self.values.next()
            .ok_or_else(||
                DeError::ExpectedValue { field: self.field.unwrap_or("<unknown>") }
            )
    }
}

/// Parse space separated values from a string
pub fn from_str<'a, T>(s: &'a str) -> Result<T, DeError>
where
    T: Deserialize<'a>,
{
    let mut deserializer = Deserializer::from_str(s);
    let t = T::deserialize(&mut deserializer)?;
    Ok(t)
}

struct SpaceSeparated<'a, 'de: 'a> {
    de: &'a mut Deserializer<'de>,
    fields: std::slice::Iter<'static, &'static str>,
}

impl<'a, 'de> SpaceSeparated<'a, 'de> {
    fn new(de: &'a mut Deserializer<'de>, fields: &'static [&'static str]) -> Self {
        Self {
            de,
            fields: fields.iter(),
        }
    }
}

impl<'a, 'de> MapAccess<'de> for SpaceSeparated<'a, 'de> {
    type Error = DeError;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, DeError>
    where
        K: DeserializeSeed<'de>,
    {
        if let Some(field) = self.fields.next() {
            self.de.field = Some(field);
            seed.deserialize(&mut *self.de).map(Some)
        } else {
            Ok(None)
        }
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, DeError>
    where
        V: DeserializeSeed<'de>,
    {
        seed.deserialize(&mut *self.de)
    }
}

impl<'de, 'a> SeqAccess<'de> for SpaceSeparated<'a, 'de> {
    type Error = DeError;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>, DeError>
    where
        T: DeserializeSeed<'de>,
    {
        seed.deserialize(&mut *self.de).map(Some)
    }
}

impl<'de, 'a> de::Deserializer<'de> for &'a mut Deserializer<'de> {
    type Error = DeError;

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_map(SpaceSeparated::new(self, fields))
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_seq(SpaceSeparated::new(self, &[]))
    }

    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        self.deserialize_seq(visitor)
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_borrowed_str(self.field()?)
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_borrowed_str(self.value()?)
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        self.deserialize_str(visitor)
    }

    fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        let v = self.value()?;
        if v.len() != 1 {
            return Err(DeError::ExpectedChar { field: self.field_or_unknown() })
        }
        visitor.visit_char(v.chars().next().unwrap())
    }

    fn deserialize_i8<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i8(
            self.value()?
                .parse()
                .context(ExpectedIntegerSnafu { field: self.field_or_unknown() })?
        )
    }

    fn deserialize_i16<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i16(
            self.value()?
                .parse()
                .context(ExpectedIntegerSnafu { field: self.field_or_unknown() })?
        )
    }

    fn deserialize_i32<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i32(
            self.value()?
                .parse()
                .context(ExpectedIntegerSnafu { field: self.field_or_unknown() })?
        )
    }

    fn deserialize_i64<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_i64(
            self.value()?
                .parse()
                .context(ExpectedIntegerSnafu { field: self.field_or_unknown() })?
        )
    }

    fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u8(
            self.value()?
                .parse()
                .context(ExpectedIntegerSnafu { field: self.field_or_unknown() })?
        )
    }

    fn deserialize_u16<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u16(
            self.value()?
                .parse()
                .context(ExpectedIntegerSnafu { field: self.field_or_unknown() })?
        )
    }

    fn deserialize_u32<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u32(
            self.value()?
                .parse()
                .context(ExpectedIntegerSnafu { field: self.field_or_unknown() })?
        )
    }

    fn deserialize_u64<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_u64(
            self.value()?
                .parse()
                .context(ExpectedIntegerSnafu { field: self.field_or_unknown() })?
        )
    }

    fn deserialize_f32<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_f32(
            self.value()?
                .parse()
                .context(ExpectedFloatSnafu { field: self.field_or_unknown() })?
        )
    }

    fn deserialize_f64<V>(self, visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        visitor.visit_f64(
            self.value()?
                .parse()
                .context(ExpectedFloatSnafu { field: self.field_or_unknown() })?
        )
    }

    // *** Unimplemented types ***

    fn deserialize_any<V>(self, _visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        Err(DeError::unsupported_type("any"))
    }

    fn deserialize_bool<V>(self, _visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        Err(DeError::unsupported_type("bool"))
    }

    fn deserialize_bytes<V>(self, _visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        Err(DeError::unsupported_type("bytes"))
    }

    fn deserialize_byte_buf<V>(self, _visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        Err(DeError::unsupported_type("byte_buf"))
    }

    fn deserialize_option<V>(self, _visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        Err(DeError::unsupported_type("option"))
    }

    fn deserialize_unit<V>(self, _visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        Err(DeError::unsupported_type("unit"))
    }

    fn deserialize_unit_struct<V>(
        self,
        _name: &'static str,
        _visitor: V,
    ) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        Err(DeError::unsupported_type("unit_struct"))
    }

    fn deserialize_newtype_struct<V>(
        self,
        _name: &'static str,
        _visitor: V,
    ) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        Err(DeError::unsupported_type("newtype_struct"))
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        _visitor: V,
    ) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        Err(DeError::unsupported_type("tuple_struct"))
    }

    fn deserialize_map<V>(self, _visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        Err(DeError::unsupported_type("map"))
    }

    fn deserialize_enum<V>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        Err(DeError::unsupported_type("enum"))
    }

    fn deserialize_ignored_any<V>(self, _visitor: V) -> Result<V::Value, DeError>
    where
        V: Visitor<'de>,
    {
        Err(DeError::unsupported_type("ignored_any"))
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use super::{DeError, from_str};

    #[derive(Deserialize, Debug, PartialEq)]
    struct Data {
        voltage: f32,
        frequency: f32,
        status: String,
    }

    #[test]
    fn test_parse_into_struct() {
        let data: Data = from_str("233.6 49.9 01001 000").unwrap();
        assert_eq!(
            data,
            Data {
                voltage: 233.6,
                frequency: 49.9,
                status: "01001".to_string(),
            }
        )
    }

    #[test]
    fn test_parse_into_struct_missing_field() {
        let res: Result<Data, DeError> = from_str("233.6 49.9");
        assert_eq!(
            res,
            Err(DeError::ExpectedValue { field: "status" })
        )
    }

    #[test]
    fn test_parse_status1_into_tuple() {
        let data: (f64, f64, String) = from_str("233.6 49.9 01001 00010000").unwrap();
        assert_eq!(
            data,
            (233.6, 49.9, "01001".to_string())
        )
    }

    #[test]
    fn test_parse_into_tuple_missing_field() {
        let res: Result<(f64, f64, u32), DeError> = from_str("233.6 49.9");
        assert_eq!(
            res,
            Err(DeError::ExpectedValue { field: "<unknown>" })
        )
    }

}
