// Copyright (c) 2024 Hemashushu <hippospark@gmail.com>, All rights reserved.
//
// This Source Code Form is subject to the terms of
// the Mozilla Public License version 2.0 and additional exceptions,
// more details in file LICENSE, LICENSE.additional and CONTRIBUTING.

use std::io::Read;

use serde::de::{self, EnumAccess, IntoDeserializer, MapAccess, SeqAccess, VariantAccess};

use crate::{
    charposition::CharsWithPositionIter,
    charstream::{CharStreamFromCharIter, CharStreamFromReader},
    error::Error,
    lexer::{NumberToken, Token, TokenIter, TokenWithRange},
    location::{Position, Range},
    normalizer::{ClearTokenIter, NormalizedTokenIter, TrimmedTokenIter},
    peekableiter::PeekableIter,
};

use super::Result;

pub fn from_str<T>(s: &str) -> Result<T>
where
    T: de::DeserializeOwned,
{
    let mut chars = s.chars();
    let mut char_stream = CharStreamFromCharIter::new(&mut chars);
    from_char_stream(&mut char_stream)
}

pub fn from_reader<T, R: Read>(mut r: R) -> Result<T>
where
    T: de::DeserializeOwned,
{
    let mut char_stream = CharStreamFromReader::new(&mut r);
    from_char_stream(&mut char_stream)
}

pub fn from_char_stream<T>(char_stream: &mut dyn Iterator<Item = Result<char>>) -> Result<T>
where
    T: de::DeserializeOwned,
{
    // There are two main ways to write Deserialize trait bounds,
    // whether on an impl block or a function or anywhere else.
    // - <'de, T> where T: Deserialize<'de>
    // - <T> where T: DeserializeOwned
    // see:
    // https://serde.rs/lifetimes.html

    let mut position_iter = CharsWithPositionIter::new(0, char_stream);
    let mut peekable_position_iter = PeekableIter::new(&mut position_iter, 3);
    let mut token_iter = TokenIter::new(&mut peekable_position_iter);

    let mut clear_iter = ClearTokenIter::new(&mut token_iter);
    let mut peekable_clear_iter = PeekableIter::new(&mut clear_iter, 1);
    let mut normalized_iter = NormalizedTokenIter::new(&mut peekable_clear_iter);
    let mut peekable_normalized_iter = PeekableIter::new(&mut normalized_iter, 1);
    let mut trimmed_iter = TrimmedTokenIter::new(&mut peekable_normalized_iter);
    let mut peekable_trimmed_iter = PeekableIter::new(&mut trimmed_iter, 2);

    let mut deserializer = Deserializer::from_token_peekable_iter(&mut peekable_trimmed_iter);
    let value = T::deserialize(&mut deserializer)?;

    match deserializer.upstream.peek(0) {
        Some(Ok(TokenWithRange { range, .. })) => Err(Error::MessageWithPosition(
            "Syntax error, the ASON document contains more than one node.".to_owned(),
            Position::from_range_start(range),
        )),
        Some(Err(e)) => Err(e.clone()),
        None => {
            // expected
            Ok(value)
        }
    }
}

pub struct Deserializer<'de> {
    upstream: &'de mut PeekableIter<'de, Result<TokenWithRange>>,
    last_range: Range,
}

impl<'de> Deserializer<'de> {
    pub fn from_token_peekable_iter(
        upstream: &'de mut PeekableIter<'de, Result<TokenWithRange>>,
    ) -> Self {
        Self {
            upstream,
            last_range: Range::new(0, 0, 0, 0, 0),
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>> {
        match self.upstream.next() {
            Some(Ok(TokenWithRange { token, range })) => {
                self.last_range = range;
                Ok(Some(token))
            }
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    fn peek_token(&self, offset: usize) -> Result<Option<&Token>> {
        match self.upstream.peek(offset) {
            Some(Ok(TokenWithRange { token, .. })) => Ok(Some(token)),
            Some(Err(e)) => Err(e.clone()),
            None => Ok(None),
        }
    }

    fn peek_range(&self, offset: usize) -> Result<Option<&Range>> {
        match self.upstream.peek(offset) {
            Some(Ok(TokenWithRange { range, .. })) => Ok(Some(range)),
            Some(Err(e)) => Err(e.clone()),
            None => Ok(None),
        }
    }

    fn peek_token_and_equals(&self, offset: usize, expected_token: &Token) -> bool {
        matches!(
            self.upstream.peek(offset),
            Some(Ok(TokenWithRange { token, .. })) if token == expected_token)
    }

    fn consume_token(&mut self, expected_token: &Token) -> Result<()> {
        match self.next_token()? {
            Some(token) => {
                if &token == expected_token {
                    Ok(())
                } else {
                    Err(Error::MessageWithPosition(
                        format!(
                            "Expect token: {:?}, actual token: {:?}",
                            expected_token, token
                        ),
                        self.last_range.get_position_start(),
                    ))
                }
            }
            None => Err(Error::MessageWithPosition(
                format!(
                    "Expect token: \"{:?}\", unexpected to reach the end of document.",
                    expected_token
                ),
                self.last_range.get_position_end(),
            )),
        }
    }

    // consume ':'
    fn consume_colon(&mut self) -> Result<()> {
        self.consume_token(&Token::Colon)
    }

    // consume ')'
    fn consume_right_paren(&mut self) -> Result<()> {
        self.consume_token(&Token::RightParen)
    }

    // consume ']'
    fn consume_right_bracket(&mut self) -> Result<()> {
        self.consume_token(&Token::RightBracket)
    }

    // consume '}'
    fn consume_right_brace(&mut self) -> Result<()> {
        self.consume_token(&Token::RightBrace)
    }

    // consume '\n' or ',' if they exist.
    fn consume_new_line_or_comma_if_exist(&mut self) -> Result<()> {
        if matches!(self.peek_token(0)?, Some(Token::NewLine | Token::Comma)) {
            self.next_token()?;
        }

        Ok(())
    }
}

impl<'de, 'a> de::Deserializer<'de> for &'a mut Deserializer<'de> {
    type Error = Error;

    fn deserialize_any<V>(self, _visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        unimplemented!()
    }

    fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::Boolean(v)) => visitor.visit_bool(v),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect a \"Boolean\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect a \"Boolean\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_i8<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::Number(NumberToken::I8(v))) => visitor.visit_i8(v as i8),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect an \"i8\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect an \"i8\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_i16<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::Number(NumberToken::I16(v))) => visitor.visit_i16(v as i16),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect an \"i16\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect an \"i16\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_i32<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::Number(NumberToken::I32(v))) => visitor.visit_i32(v as i32),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect an \"i32\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect an \"i32\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_i64<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::Number(NumberToken::I64(v))) => visitor.visit_i64(v as i64),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect an \"i64\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect an \"i64\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::Number(NumberToken::U8(v))) => visitor.visit_u8(v),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect an \"u8\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect an \"u8\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_u16<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::Number(NumberToken::U16(v))) => visitor.visit_u16(v),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect an \"u16\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect an \"u16\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_u32<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::Number(NumberToken::U32(v))) => visitor.visit_u32(v),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect an \"u32\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect an \"u32\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_u64<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::Number(NumberToken::U64(v))) => visitor.visit_u64(v),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect an \"u64\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect an \"u64\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_f32<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::Number(NumberToken::F32(v))) => visitor.visit_f32(v),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect a \"f32\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect a \"f32\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_f64<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::Number(NumberToken::F64(v))) => visitor.visit_f64(v),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect a \"f64\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect a \"f64\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_char<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::Char(c)) => visitor.visit_char(c),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect a \"Char\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect a \"Char\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_str<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::String_(s)) => visitor.visit_str(&s),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect a \"String\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect a \"String\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_string<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::String_(s)) => visitor.visit_string(s),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect a \"String\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect a \"String\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::ByteData(d)) => visitor.visit_bytes(&d),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect a \"Bytes\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect a \"Bytes\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::ByteData(d)) => visitor.visit_byte_buf(d),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect a \"Bytes\" value.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect a \"Bytes\" value, unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_option<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::Variant(type_name, member_name)) => {
                if type_name == "Option" {
                    if member_name == "None" && !self.peek_token_and_equals(0, &Token::LeftParen) {
                        visitor.visit_none()
                    } else if member_name == "Some"
                        && self.peek_token_and_equals(0, &Token::LeftParen)
                    {
                        self.next_token()?; // consume '('
                        let v = visitor.visit_some(&mut *self);
                        self.consume_right_paren()?;
                        v
                    } else {
                        Err(Error::MessageWithRange(
                            format!(
                                "Unexpected member \"{}\" of variant \"Option\".",
                                member_name
                            ),
                            *self.peek_range(0)?.unwrap(),
                        ))
                    }
                } else {
                    Err(Error::MessageWithRange(
                        format!(
                            "Expect \"Option\" type of variant, actual: \"{}\".",
                            type_name
                        ),
                        self.last_range,
                    ))
                }
            }
            Some(_) => Err(Error::MessageWithPosition(
                "Expect variant \"Option\".".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect variant \"Option\", unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_unit<V>(self, _visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        // The type of `()` in Rust.
        // It represents an anonymous value containing no data.
        Err(Error::Message("ASON does not support \"Unit\".".to_owned()))
    }

    fn deserialize_unit_struct<V>(self, _name: &'static str, _visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        // For example `struct Unit` or `PhantomData<T>`.
        // It represents a named value containing no data.
        Err(Error::Message(
            "ASON does not support \"Unit Struct\".".to_owned(),
        ))
    }

    fn deserialize_newtype_struct<V>(self, _name: &'static str, _visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        // For example `struct Millimeters(u8)`.
        Err(Error::Message(
            "ASON does not support \"New Type Struct\".".to_owned(),
        ))
    }

    fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        // seq = List

        match self.next_token()? {
            Some(Token::LeftBracket) => {
                let value = visitor.visit_seq(ArrayAccessor::new(self))?;

                self.consume_right_bracket()?; // consume ']'
                self.consume_new_line_or_comma_if_exist()?; // consume trailing newlines

                Ok(value)
            }
            Some(_) => Err(Error::MessageWithPosition(
                "Expect a \"List\".".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect a \"List\", unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_tuple<V>(self, _len: usize, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        match self.next_token()? {
            Some(Token::LeftParen) => {
                let value = visitor.visit_seq(TupleAccessor::new(self))?;

                self.consume_new_line_or_comma_if_exist()?; // consume additional newlines
                self.consume_right_paren()?; // consume ')'
                self.consume_new_line_or_comma_if_exist()?; // consume trailing newlines

                Ok(value)
            }
            Some(_) => Err(Error::MessageWithPosition(
                "Expect a \"Tuple\".".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect a \"Tuple\", unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_tuple_struct<V>(
        self,
        _name: &'static str,
        _len: usize,
        _visitor: V,
    ) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        // A named tuple, for example `struct Rgb(u8, u8, u8)`.
        Err(Error::Message(
            "ASON does not support \"Tuple Struct\".".to_owned(),
        ))
    }

    fn deserialize_map<V>(self, _visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        // A variably sized heterogeneous key-value pairing,
        // for example `BTreeMap<K, V>`.
        // When serializing, the length may or may not be known before
        // iterating through all the entries. When deserializing,
        // the length is determined by looking at the serialized data.
        Err(Error::Message("ASON does not support Map.".to_owned()))
    }

    fn deserialize_struct<V>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        // struct = Object

        match self.next_token()? {
            Some(Token::LeftBrace) => {
                let value = visitor.visit_map(ObjectAccessor::new(self))?;

                self.consume_new_line_or_comma_if_exist()?; // consume additional newlines
                self.consume_right_brace()?; // consume '}'
                self.consume_new_line_or_comma_if_exist()?; // consume trailing newlines

                Ok(value)
            }
            Some(_) => Err(Error::MessageWithPosition(
                "Expect an \"Object\".".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect an \"Object\", unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_enum<V>(
        self,
        name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        // enum = Variant
        match self.next_token()? {
            Some(Token::Variant(type_name, member_name)) => {
                if type_name == name {
                    if self.peek_token_and_equals(0, &Token::LeftParen) {
                        // variant with single value or multiple values
                        let v = visitor.visit_enum(VariantAccessor::new(self, &member_name))?;
                        Ok(v)
                    } else if self.peek_token_and_equals(0, &Token::LeftBrace) {
                        // variant with struct value
                        let v = visitor.visit_enum(VariantAccessor::new(self, &member_name))?;
                        Ok(v)
                    } else {
                        // variant without value
                        visitor.visit_enum(member_name.into_deserializer())
                    }
                } else {
                    Err(Error::MessageWithRange(
                        format!(
                            "Expect type: \"{}\" of variant, actual: \"{}\".",
                            name, type_name
                        ),
                        self.last_range,
                    ))
                }
            }
            Some(_) => Err(Error::MessageWithPosition(
                "Expect a \"Variant\".".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect a \"Variant\", unexpected to reach the end of document.".to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        // An identifier in Serde is the type that identifies a field of a struct.
        // In ASON, struct fields  represented as strings.

        match self.next_token()? {
            Some(Token::Identifier(id)) => visitor.visit_string(id),
            Some(_) => Err(Error::MessageWithPosition(
                "Expect an identifier for object.".to_owned(),
                self.last_range.get_position_start(),
            )),
            None => Err(Error::MessageWithPosition(
                "Expect an identifier for object, unexpected to reach the end of document."
                    .to_owned(),
                self.last_range.get_position_end(),
            )),
        }
    }

    fn deserialize_ignored_any<V>(self, _visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        unimplemented!()
    }
}

struct ArrayAccessor<'a, 'de: 'a> {
    de: &'a mut Deserializer<'de>,
}

impl<'a, 'de> ArrayAccessor<'a, 'de> {
    fn new(de: &'a mut Deserializer<'de>) -> Self {
        Self { de }
    }
}

impl<'de, 'a> SeqAccess<'de> for ArrayAccessor<'a, 'de> {
    type Error = Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where
        T: de::DeserializeSeed<'de>,
    {
        self.de.consume_new_line_or_comma_if_exist()?;

        if self.de.peek_token_and_equals(0, &Token::RightBracket) {
            // exits the procedure when the end marker ']' is encountered.
            return Ok(None);
        }

        seed.deserialize(&mut *self.de).map(Some)
    }
}

struct TupleAccessor<'a, 'de: 'a> {
    de: &'a mut Deserializer<'de>,
}

impl<'a, 'de> TupleAccessor<'a, 'de> {
    fn new(de: &'a mut Deserializer<'de>) -> Self {
        Self { de }
    }
}

impl<'de, 'a> SeqAccess<'de> for TupleAccessor<'a, 'de> {
    type Error = Error;

    fn next_element_seed<T>(&mut self, seed: T) -> Result<Option<T::Value>>
    where
        T: de::DeserializeSeed<'de>,
    {
        self.de.consume_new_line_or_comma_if_exist()?;
        seed.deserialize(&mut *self.de).map(Some)

        // the deserializer knows the number of members of the
        // target tuple, so it doesn't need to check the
        // ending marker ')'.
    }
}

struct VariantAccessor<'a, 'de: 'a> {
    de: &'a mut Deserializer<'de>,
    variant_member_name: &'a str,
}

impl<'a, 'de> VariantAccessor<'a, 'de> {
    fn new(de: &'a mut Deserializer<'de>, variant_member_name: &'a str) -> Self {
        Self {
            de,
            variant_member_name,
        }
    }
}

// `EnumAccess` is provided to the `Visitor` to give it the ability to determine
// which variant of the enum is supposed to be deserialized.
//
// Note that all enum deserialization methods in Serde refer exclusively to the
// "externally tagged" enum representation.
impl<'de, 'a> EnumAccess<'de> for VariantAccessor<'a, 'de> {
    type Error = Error;
    type Variant = Self;

    fn variant_seed<V>(self, seed: V) -> Result<(V::Value, Self::Variant)>
    where
        V: de::DeserializeSeed<'de>,
    {
        let value = seed.deserialize(self.variant_member_name.into_deserializer())?;
        Ok((value, self))
    }
}

// `VariantAccess` is provided to the `Visitor` to give it the ability to see
// the content of the single variant that it decided to deserialize.
impl<'de, 'a> VariantAccess<'de> for VariantAccessor<'a, 'de> {
    type Error = Error;

    // If the `Visitor` expected this variant to be a unit variant, the input
    // should have been the plain string case handled in `deserialize_enum`.
    fn unit_variant(self) -> Result<()> {
        unreachable!()
    }

    // Newtype variants are represented in ASON as `(value)` so
    // deserialize the value here.
    fn newtype_variant_seed<T>(self, seed: T) -> Result<T::Value>
    where
        T: de::DeserializeSeed<'de>,
    {
        self.de.next_token()?; // consume '('
        self.de.consume_new_line_or_comma_if_exist()?;

        let v = seed.deserialize(&mut *self.de);
        self.de.consume_new_line_or_comma_if_exist()?;

        self.de.next_token()?; // consume ')'
        self.de.consume_new_line_or_comma_if_exist()?;
        v
    }

    fn tuple_variant<V>(self, len: usize, visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        de::Deserializer::deserialize_tuple(self.de, len, visitor)
    }

    fn struct_variant<V>(self, fields: &'static [&'static str], visitor: V) -> Result<V::Value>
    where
        V: de::Visitor<'de>,
    {
        de::Deserializer::deserialize_struct(self.de, "", fields, visitor)
    }
}

struct ObjectAccessor<'a, 'de: 'a> {
    de: &'a mut Deserializer<'de>,
}

impl<'a, 'de> ObjectAccessor<'a, 'de> {
    fn new(de: &'a mut Deserializer<'de>) -> Self {
        Self { de }
    }
}

impl<'de, 'a> MapAccess<'de> for ObjectAccessor<'a, 'de> {
    type Error = Error;

    fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>>
    where
        K: de::DeserializeSeed<'de>,
    {
        self.de.consume_new_line_or_comma_if_exist()?;

        // it seems the struct/object accessor wouldn't stop automatically when
        // it encounters the last field.
        if self.de.peek_token_and_equals(0, &Token::RightBrace) {
            return Ok(None);
        }

        // Deserialize a field key.
        seed.deserialize(&mut *self.de).map(Some)

        // the function 'deserialize_identifier' is called here, and then
        // the key name will be obtained.
    }

    fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value>
    where
        V: de::DeserializeSeed<'de>,
    {
        self.de.consume_new_line_or_comma_if_exist()?;
        self.de.consume_colon()?;
        self.de.consume_new_line_or_comma_if_exist()?;

        // Deserialize a field value.
        seed.deserialize(&mut *self.de)
    }
}

#[cfg(test)]
mod tests {
    use crate::serde::de::from_str;

    use pretty_assertions::assert_eq;
    use serde::Deserialize;
    use serde_bytes::ByteBuf;

    #[test]
    fn test_primitive_types() {
        // bool
        {
            assert_eq!(from_str::<bool>(r#"true"#).unwrap(), true);
            assert_eq!(from_str::<bool>(r#"false"#).unwrap(), false);
        }

        // signed integers
        {
            assert_eq!(from_str::<i8>(r#"11_i8"#).unwrap(), 11);
            assert_eq!(from_str::<i16>(r#"13_i16"#).unwrap(), 13);
            assert_eq!(from_str::<i32>(r#"17"#).unwrap(), 17);
            assert_eq!(from_str::<i32>(r#"17_i32"#).unwrap(), 17);
            assert_eq!(from_str::<i64>(r#"19_i64"#).unwrap(), 19);
        }

        // unsigned integers
        {
            assert_eq!(from_str::<u8>(r#"11_u8"#).unwrap(), 11);
            assert_eq!(from_str::<u16>(r#"13_u16"#).unwrap(), 13);
            assert_eq!(from_str::<u32>(r#"17_u32"#).unwrap(), 17);
            assert_eq!(from_str::<u64>(r#"19_u64"#).unwrap(), 19);
        }

        // f32
        {
            assert_eq!(from_str::<f32>(r#"123_f32"#).unwrap(), 123_f32);
            assert_eq!(from_str::<f32>(r#"-4.56_f32"#).unwrap(), -4.56_f32);
            assert_eq!(
                from_str::<f32>(r#"3.1415927_f32"#).unwrap(),
                std::f32::consts::PI
            );
            assert_eq!(from_str::<f32>(r#"0_f32"#).unwrap(), 0_f32);
            assert_eq!(from_str::<f32>(r#"-0_f32"#).unwrap(), 0_f32); // -0 == 0
            assert!(from_str::<f32>(r#"NaN_f32"#).unwrap().is_nan()); // NaN != NaN
            assert_eq!(from_str::<f32>(r#"Inf_f32"#).unwrap(), f32::INFINITY);
            assert_eq!(from_str::<f32>(r#"-Inf_f32"#).unwrap(), f32::NEG_INFINITY);
        }

        // f64
        {
            assert_eq!(from_str::<f64>(r#"123.0"#).unwrap(), 123_f64);
            assert_eq!(from_str::<f64>(r#"123_f64"#).unwrap(), 123_f64);
            assert_eq!(from_str::<f64>(r#"-4.56"#).unwrap(), -4.56_f64);
            assert_eq!(
                from_str::<f64>(r#"3.141592653589793"#).unwrap(),
                std::f64::consts::PI
            );
            assert_eq!(from_str::<f64>(r#"0_f64"#).unwrap(), 0_f64);
            assert_eq!(from_str::<f64>(r#"-0_f64"#).unwrap(), 0_f64); // -0 == 0
            assert!(from_str::<f64>(r#"NaN"#).unwrap().is_nan()); // NaN != NaN
            assert!(from_str::<f64>(r#"NaN_f64"#).unwrap().is_nan()); // NaN != NaN
            assert_eq!(from_str::<f64>(r#"Inf"#).unwrap(), f64::INFINITY);
            assert_eq!(from_str::<f64>(r#"-Inf"#).unwrap(), f64::NEG_INFINITY);
            assert_eq!(from_str::<f64>(r#"Inf_f64"#).unwrap(), f64::INFINITY);
            assert_eq!(from_str::<f64>(r#"-Inf_f64"#).unwrap(), f64::NEG_INFINITY);
        }

        // char
        {
            assert_eq!(from_str::<char>(r#"'a'"#).unwrap(), 'a');
            assert_eq!(from_str::<char>(r#"'文'"#).unwrap(), '文');
            assert_eq!(from_str::<char>(r#"'🍒'"#).unwrap(), '🍒');
            assert_eq!(from_str::<char>(r#"'\\'"#).unwrap(), '\\');
            assert_eq!(from_str::<char>(r#"'\''"#).unwrap(), '\'');
            assert_eq!(from_str::<char>(r#"'\"'"#).unwrap(), '"');
            assert_eq!(from_str::<char>(r#"'\t'"#).unwrap(), '\t');
            assert_eq!(from_str::<char>(r#"'\r'"#).unwrap(), '\r');
            assert_eq!(from_str::<char>(r#"'\n'"#).unwrap(), '\n');
            assert_eq!(from_str::<char>(r#"'\0'"#).unwrap(), '\0');
            assert_eq!(from_str::<char>(r#"'\u{8431}'"#).unwrap(), '萱');
        }

        // string
        {
            assert_eq!(
                from_str::<String>(r#""abc文字🍒""#).unwrap(),
                "abc文字🍒".to_owned()
            );
            assert_eq!(
                from_str::<String>(r#""abc\"\\\t\0xyz""#).unwrap(),
                "abc\"\\\t\0xyz".to_owned()
            );
            assert_eq!(
                from_str::<String>(r#""hello\nworld""#).unwrap(),
                "hello\nworld".to_owned()
            );
            assert_eq!(
                from_str::<String>(r#""\u{5c0f}\u{8431}脚本""#).unwrap(),
                "小萱脚本".to_owned()
            );

            assert_eq!(
                from_str::<String>(
                    r#"
            r"a\nb"
            "#
                )
                .unwrap(),
                "a\\nb".to_owned()
            );

            assert_eq!(
                from_str::<String>(
                    r#"
            """
            a
            \tb
                c
            """
            "#
                )
                .unwrap(),
                "a\n\\tb\n    c".to_owned()
            );
        }
    }

    #[test]
    fn test_byte_data() {
        assert_eq!(
            from_str::<ByteBuf>(r#"h"0b 0d 11 13""#).unwrap(),
            ByteBuf::from(vec![11u8, 13, 17, 19])
        );

        assert_eq!(
            from_str::<ByteBuf>(r#"h"61 62 63""#).unwrap(),
            ByteBuf::from(b"abc")
        );
    }

    #[test]
    fn test_option() {
        assert_eq!(from_str::<Option<i32>>(r#"Option::None"#).unwrap(), None);
        assert_eq!(
            from_str::<Option<i32>>(r#"Option::Some(123)"#).unwrap(),
            Some(123)
        );
    }

    #[test]
    fn test_list() {
        assert_eq!(
            from_str::<Vec<i32>>(r#"[11,13,17,19]"#).unwrap(),
            vec![11, 13, 17, 19]
        );

        assert_eq!(
            from_str::<Vec<u8>>(
                r#"[
    97_u8
    98_u8
    99_u8
]"#
            )
            .unwrap(),
            b"abc"
        );

        assert_eq!(
            from_str::<Vec<String>>(
                r#"[
    "foo"
    "bar"
    "2024"
]"#
            )
            .unwrap(),
            vec!["foo", "bar", "2024"]
        );

        // nested list
        assert_eq!(
            from_str::<Vec<Vec<i32>>>(
                r#"[
    [11,13]
    [17,19]
    [23,29]
]"#
            )
            .unwrap(),
            vec![vec![11, 13], vec![17, 19], vec![23, 29]]
        );
    }

    #[test]
    fn test_tuple() {
        assert_eq!(
            from_str::<(i32, i32, i32, i32)>(r#"(11, 13, 17, 19)"#).unwrap(),
            (11, 13, 17, 19)
        );

        // a fixed-length array is treated as tuple
        assert_eq!(
            from_str::<[u8; 3]>(
                r#"(
97_u8
98_u8
99_u8
)"#
            )
            .unwrap(),
            b"abc".to_owned()
        );

        assert_eq!(
            from_str::<(String, String, String)>(
                r#"(
"foo", "bar", "2024", )"#
            )
            .unwrap(),
            ("foo".to_owned(), "bar".to_owned(), "2024".to_owned())
        );

        // nested tuple
        assert_eq!(
            from_str::<((i32, i32), (i32, i32), (i32, i32))>(r#"((11, 13), (17, 19), (23, 29))"#)
                .unwrap(),
            ((11, 13), (17, 19), (23, 29))
        );
    }

    #[test]
    fn test_object() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Object {
            id: i32,
            name: String,
            checked: bool,
        }

        assert_eq!(
            from_str::<Object>(r#"{id: 123, name: "foo", checked: true}"#).unwrap(),
            Object {
                id: 123,
                name: "foo".to_owned(),
                checked: true
            }
        );

        // nested object
        #[derive(Deserialize, Debug, PartialEq)]
        struct Address {
            code: i32,
            city: String,
        }

        #[derive(Deserialize, Debug, PartialEq)]
        struct NestedObject {
            id: i32,
            name: String,
            address: Box<Address>,
        }

        assert_eq!(
            from_str::<NestedObject>(
                r#"{
    id: 456
    name: "bar"
    address: {
        code: 518000
        city: "sz"
    }
}"#
            )
            .unwrap(),
            NestedObject {
                id: 456,
                name: "bar".to_owned(),
                address: Box::new(Address {
                    code: 518000,
                    city: "sz".to_owned()
                })
            }
        )
    }

    #[test]
    fn test_variant_with_value() {
        #[derive(Deserialize, Debug, PartialEq)]
        enum Color {
            Red,
            Green,
            Blue,
            Grey(u8),
        }

        assert_eq!(from_str::<Color>(r#"Color::Red"#).unwrap(), Color::Red);
        assert_eq!(from_str::<Color>(r#"Color::Green"#).unwrap(), Color::Green);
        assert_eq!(
            from_str::<Color>(r#"Color::Grey(11_u8)"#).unwrap(),
            Color::Grey(11)
        );

        // nested
        #[derive(Deserialize, Debug, PartialEq)]
        enum Apperance {
            Transparent,
            Color(Color),
        }

        assert_eq!(
            from_str::<Apperance>(r#"Apperance::Transparent"#).unwrap(),
            Apperance::Transparent
        );

        assert_eq!(
            from_str::<Apperance>(r#"Apperance::Color(Color::Blue)"#).unwrap(),
            Apperance::Color(Color::Blue)
        );

        assert_eq!(
            from_str::<Apperance>(r#"Apperance::Color(Color::Grey(13_u8))"#).unwrap(),
            Apperance::Color(Color::Grey(13))
        );
    }

    #[test]
    fn test_variant_with_tuple_values() {
        #[allow(clippy::upper_case_acronyms)]
        #[derive(Deserialize, Debug, PartialEq)]
        enum Color {
            RGB(u8, u8, u8),
            Grey(u8),
        }

        assert_eq!(
            from_str::<Color>(r#"Color::RGB(255_u8,127_u8,63_u8)"#).unwrap(),
            Color::RGB(255, 127, 63)
        );
    }

    #[test]
    fn test_variant_with_object_values() {
        #[derive(Deserialize, Debug, PartialEq)]
        enum Shape {
            Circle(i32),
            Rect { width: i32, height: i32 },
        }

        assert_eq!(
            from_str::<Shape>(
                r#"Shape::Rect{
    width: 200
    height: 100
}"#
            )
            .unwrap(),
            Shape::Rect {
                width: 200,
                height: 100
            }
        );

        assert_eq!(
            from_str::<Shape>(r#"Shape::Circle(127)"#).unwrap(),
            Shape::Circle(127)
        );
    }

    #[test]
    fn test_mix_list_and_tuple() {
        assert_eq!(
            from_str::<Vec<(i32, String)>>(
                r#"[
    (1, "foo")
    (2, "bar")
]"#
            )
            .unwrap(),
            vec![(1, "foo".to_owned()), (2, "bar".to_owned())]
        );

        assert_eq!(
            from_str::<(Vec<i32>, Vec<String>)>(
                r#"([
    11
    13
], [
    "foo"
    "bar"
])"#
            )
            .unwrap(),
            (vec![11, 13], vec!["foo".to_owned(), "bar".to_owned()])
        );
    }

    #[test]
    fn test_mix_list_and_object() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Object {
            id: i32,
            name: String,
        }

        #[derive(Deserialize, Debug, PartialEq)]
        struct ObjectList {
            id: i32,
            items: Vec<i32>,
        }

        assert_eq!(
            from_str::<Vec<Object>>(
                r#"[
    {
        id: 11
        name: "foo"
    }
    {
        id: 13
        name: "bar"
    }
]"#
            )
            .unwrap(),
            vec![
                Object {
                    id: 11,
                    name: "foo".to_owned()
                },
                Object {
                    id: 13,
                    name: "bar".to_owned()
                }
            ]
        );

        assert_eq!(
            from_str::<ObjectList>(
                r#"{
    id: 456
    items: [
        11
        13
        17
        19
    ]
}"#
            )
            .unwrap(),
            ObjectList {
                id: 456,
                items: vec![11, 13, 17, 19]
            }
        );
    }

    #[test]
    fn test_mix_tuple_and_object() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Object {
            id: i32,
            name: String,
        }

        #[derive(Deserialize, Debug, PartialEq)]
        struct ObjectDetail {
            id: i32,
            address: (i32, String),
        }

        assert_eq!(
            from_str::<(i32, Object)>(
                r#"(123, {
                id: 11
                name: "foo"
            })"#
            )
            .unwrap(),
            (
                123,
                Object {
                    id: 11,
                    name: "foo".to_owned()
                }
            )
        );

        assert_eq!(
            from_str::<ObjectDetail>(
                r#"{
    id: 456
    address: (11, "sz")
}"#
            )
            .unwrap(),
            ObjectDetail {
                id: 456,
                address: (11, "sz".to_owned())
            }
        );
    }

    #[test]
    fn test_mix_variant_object() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Simple {
            id: i32,
            name: String,
        }

        #[derive(Deserialize, Debug, PartialEq)]
        struct Complex {
            id: i32,
            checked: bool,
            fullname: String,
        }

        #[derive(Deserialize, Debug, PartialEq)]
        enum Item {
            Empty,
            Minimal(i32),
            Simple(Simple),
            Complex(Complex),
        }

        assert_eq!(
            from_str::<Vec<Item>>(
                r#"[
    Item::Empty
    Item::Minimal(11)
    Item::Simple({
        id: 13
        name: "foo"
    })
    Item::Complex({
        id: 17
        checked: true
        fullname: "foobar"
    })
]"#
            )
            .unwrap(),
            vec![
                Item::Empty,
                Item::Minimal(11),
                Item::Simple(Simple {
                    id: 13,
                    name: "foo".to_owned()
                }),
                Item::Complex(Complex {
                    id: 17,
                    checked: true,
                    fullname: "foobar".to_owned()
                })
            ]
        );
    }
}
