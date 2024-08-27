// Copyright (c) 2024 Hemashushu <hippospark@gmail.com>, All rights reserved.
//
// This Source Code Form is subject to the terms of
// the Mozilla Public License version 2.0 and additional exceptions,
// more details in file LICENSE, LICENSE.additional and CONTRIBUTING.

use crate::{
    error::Error,
    lexer::{CharsWithPositionIter, NumberToken, Token, TokenIter, TokenWithRange},
    location::{Position, Range},
    normalizer::{ClearTokenIter, NormalizedTokenIter, TrimmedTokenIter},
    peekableiter::PeekableIter,
};

use super::{AsonNode, KeyValuePair, Number, Variant};

pub fn parse_from(s: &str) -> Result<AsonNode, Error> {
    let mut chars = s.chars();
    let mut char_position_iter = CharsWithPositionIter::new(0, &mut chars);
    let mut peekable_char_position_iter = PeekableIter::new(&mut char_position_iter, 3);
    let mut token_iter = TokenIter::new(&mut peekable_char_position_iter);
    let mut clear_iter = ClearTokenIter::new(&mut token_iter);
    let mut peekable_clear_iter = PeekableIter::new(&mut clear_iter, 1);
    let mut normalized_iter = NormalizedTokenIter::new(&mut peekable_clear_iter);
    let mut peekable_normalized_iter = PeekableIter::new(&mut normalized_iter, 1);
    let mut trimmed_iter = TrimmedTokenIter::new(&mut peekable_normalized_iter);
    let mut peekable_trimmed_iter = PeekableIter::new(&mut trimmed_iter, 2);

    let mut parser = Parser::new(&mut peekable_trimmed_iter);
    let root = parser.parse_node()?;

    // check trailing token
    match parser.next_token()? {
        Some(_) => Err(Error::MessageWithPosition(
            "The ASON document does not end properly.".to_owned(),
            parser.get_position_from_range_start(),
        )),
        None => Ok(root),
    }
}

struct Parser<'a> {
    upstream: &'a mut PeekableIter<'a, Result<TokenWithRange, Error>>,
    current_range: Range,
}

impl<'a> Parser<'a> {
    fn new(upstream: &'a mut PeekableIter<'a, Result<TokenWithRange, Error>>) -> Self {
        Self {
            upstream,
            current_range: Range::new(0, 0, 0, 0, 0),
        }
    }

    fn next_token(&mut self) -> Result<Option<Token>, Error> {
        match self.upstream.next() {
            Some(Ok(TokenWithRange { token, range })) => {
                self.current_range = range;
                Ok(Some(token))
            }
            Some(Err(e)) => Err(e),
            None => Ok(None),
        }
    }

    fn peek_token(&self, offset: usize) -> Result<Option<&Token>, Error> {
        match self.upstream.peek(offset) {
            Some(Ok(TokenWithRange { token, .. })) => Ok(Some(token)),
            Some(Err(e)) => Err(e.clone()),
            None => Ok(None),
        }
    }

    fn get_position_from_range_start(&self) -> Position {
        Position::from_range_start(&self.current_range)
    }

    fn get_position_from_range_end(&self) -> Position {
        Position::from_range_end(&self.current_range)
    }

    fn consume_token(&mut self, expect_token: &Token) -> Result<(), Error> {
        match self.next_token()? {
            Some(token) => {
                if &token == expect_token {
                    Ok(())
                } else {
                    Err(Error::MessageWithPosition(
                        format!(
                            "Expect token: {:?}, actual token: {:?}",
                            expect_token, token
                        ),
                        self.get_position_from_range_start(),
                    ))
                }
            }
            None => Err(Error::MessageWithPosition(
                format!("Missing token: {:?}", expect_token),
                self.get_position_from_range_end(),
            )),
        }
    }

    // consume ':'
    fn consume_colon(&mut self) -> Result<(), Error> {
        self.consume_token(&Token::Colon)
    }

    // consume '\n' or ',' if they exist.
    fn consume_new_line_or_comma_if_exist(&mut self) -> Result<(), Error> {
        if matches!(self.peek_token(0)?, Some(Token::NewLine | Token::Comma)) {
            self.next_token()?;
        }

        Ok(())
    }
}

impl<'a> Parser<'a> {
    fn parse_node(&mut self) -> Result<AsonNode, Error> {
        loop {
            match self.peek_token(0)? {
                Some(current_token) => {
                    let node = match current_token {
                        Token::NewLine => {
                            // it is possible to exist a newline token after the ']', '}' and ')' punctuations.
                            self.next_token()?;
                            continue;
                        }
                        Token::Comma if !matches!(self.peek_token(1)?, Some(Token::Comma)) => {
                            // it is possible to exist a comma token after the ']', '}' and ')' punctuations.
                            self.next_token()?;
                            continue;
                        }
                        Token::Number(n) => {
                            let v = convert_number_token(*n);
                            self.next_token()?;
                            v
                        }
                        Token::Boolean(b) => {
                            let v = AsonNode::Boolean(*b);
                            self.next_token()?;
                            v
                        }
                        Token::Char(c) => {
                            let v = AsonNode::Char(*c);
                            self.next_token()?;
                            v
                        }
                        Token::String_(s) => {
                            let v = AsonNode::String_(s.to_owned());
                            self.next_token()?;
                            v
                        }
                        Token::Date(d) => {
                            let v = AsonNode::DateTime(*d);
                            self.next_token()?;
                            v
                        }
                        Token::Variant(type_name, member_name) => {
                            match self.peek_token(1)? {
                                Some(Token::LeftParen) => {
                                    // tuple variant or the new type variant (i.e. single value variant)
                                    self.parse_tuple_variant()?
                                }
                                Some(Token::LeftBrace) => {
                                    // struct variant
                                    self.parse_struct_variant()?
                                }
                                _ => {
                                    // unit variant
                                    let v = AsonNode::Variant(Variant::new(type_name, member_name));
                                    self.next_token()?;
                                    v
                                }
                            }
                        }
                        Token::ByteData(b) => {
                            let v = AsonNode::ByteData(b.to_owned());
                            self.next_token()?;
                            v
                        }
                        Token::LeftBrace => {
                            // object: {...}
                            self.parse_object()?
                        }
                        Token::LeftBracket => {
                            // array: [...]
                            self.parse_array()?
                        }
                        Token::LeftParen => {
                            // tuple: (...)
                            self.parse_tuple()?
                        }
                        _ => {
                            return Err(Error::MessageWithPosition(
                                "Syntax error, unexpected token.".to_owned(),
                                self.get_position_from_range_start(),
                            ))
                        }
                    };

                    return Ok(node);
                }
                None => {
                    return Err(Error::Message(
                        "Incomplete document, unexpected to reach the end of document.".to_owned(),
                    ));
                }
            }
        }
    }

    fn parse_tuple_variant(&mut self) -> Result<AsonNode, Error> {
        // type::member(...)?  //
        // ^           ^    ^__// to here
        // |           |-------// left paren, validated
        // |-------------------// current token, validated

        // consume variant token
        let (type_name, member_name) =
            if let Some(Token::Variant(type_name, member_name)) = self.next_token()? {
                (type_name, member_name)
            } else {
                unreachable!()
            };

        self.next_token()?; // consume '('

        let mut items = vec![];

        loop {
            self.consume_new_line_or_comma_if_exist()?;

            if matches!(self.peek_token(0)?, Some(Token::RightParen)) {
                break;
            }

            let value = self.parse_node()?;
            items.push(value);
        }

        self.next_token()?; // consume ')'

        let variant_item = match items.len() {
            0 => {
                return Err(Error::MessageWithPosition(
                    "The values of tuple variant can not be empty.".to_owned(),
                    self.get_position_from_range_start(),
                ));
            }
            1 => Variant::with_value(&type_name, &member_name, items.remove(0)),
            _ => Variant::with_values(&type_name, &member_name, items),
        };

        Ok(AsonNode::Variant(variant_item))
    }

    fn parse_struct_variant(&mut self) -> Result<AsonNode, Error> {
        // type::member{...}?  //
        // ^           ^    ^__// to here
        // |           |_______// left brace, validated
        // |-------------------// current token, validated

        // consume variant token
        let (type_name, member_name) =
            if let Some(Token::Variant(type_name, member_name)) = self.next_token()? {
                (type_name, member_name)
            } else {
                unreachable!()
            };

        let kvps = self.parse_key_value_pairs()?;

        Ok(AsonNode::Variant(Variant::with_object(
            &type_name,
            &member_name,
            kvps,
        )))
    }

    fn parse_key_value_pairs(&mut self) -> Result<Vec<KeyValuePair>, Error> {
        // {...}?  //
        // ^    ^__// to here
        // |-------// current token, validated

        self.next_token()?; // consume '{'

        let mut kvps: Vec<KeyValuePair> = vec![];

        loop {
            self.consume_new_line_or_comma_if_exist()?;

            if matches!(self.peek_token(0)?, Some(Token::RightBrace)) {
                break;
            }

            let name = match self.next_token()? {
                Some(Token::Identifier(n)) => n,
                Some(_) => {
                    return Err(Error::MessageWithPosition(
                        "Expect a key name for the object.".to_owned(),
                        self.get_position_from_range_start(),
                    ));
                }
                None => {
                    return Err(Error::MessageWithPosition(
                        "Incomplete object, unexpected to reach the end of document.".to_owned(),
                        self.get_position_from_range_end(),
                    ));
                }
            };

            self.consume_new_line_or_comma_if_exist()?;
            self.consume_colon()?;
            self.consume_new_line_or_comma_if_exist()?;

            let value = self.parse_node()?;
            let name_value_pair = KeyValuePair {
                key: name,
                value: Box::new(value),
            };
            kvps.push(name_value_pair);
        }

        self.next_token()?; // consume '}'

        Ok(kvps)
    }

    fn parse_object(&mut self) -> Result<AsonNode, Error> {
        let kvps = self.parse_key_value_pairs()?;
        Ok(AsonNode::Object(kvps))
    }

    fn parse_array(&mut self) -> Result<AsonNode, Error> {
        // [...]?  //
        // ^    ^__// to here
        // |-------// current token, validated

        self.next_token()?; // consume '['

        let mut items: Vec<AsonNode> = vec![];

        loop {
            self.consume_new_line_or_comma_if_exist()?;

            if matches!(self.peek_token(0)?, Some(Token::RightBracket)) {
                break;
            }

            let value = self.parse_node()?;
            items.push(value);
        }

        self.next_token()?; // consume ']'

        Ok(AsonNode::List(items))
    }

    fn parse_tuple(&mut self) -> Result<AsonNode, Error> {
        // (...)?  //
        // ^    ^__// to here
        // |-------// current token, validated

        self.next_token()?; // consume '('

        let mut items: Vec<AsonNode> = vec![];

        loop {
            self.consume_new_line_or_comma_if_exist()?;

            if matches!(self.peek_token(0)?, Some(Token::RightParen)) {
                break;
            }

            let value = self.parse_node()?;
            items.push(value);
        }

        self.next_token()?; // consume ')'

        if items.is_empty() {
            Err(Error::MessageWithPosition(
                "Tuple can not be empty.".to_owned(),
                self.get_position_from_range_start(),
            ))
        } else {
            Ok(AsonNode::Tuple(items))
        }
    }
}

fn convert_number_token(token: NumberToken) -> AsonNode {
    let number = match token {
        NumberToken::I8(v) => Number::I8(v as i8),
        NumberToken::U8(v) => Number::U8(v),
        NumberToken::I16(v) => Number::I16(v as i16),
        NumberToken::U16(v) => Number::U16(v),
        NumberToken::I32(v) => Number::I32(v as i32),
        NumberToken::U32(v) => Number::U32(v),
        NumberToken::I64(v) => Number::I64(v as i64),
        NumberToken::U64(v) => Number::U64(v),
        NumberToken::F32(v) => Number::F32(v),
        NumberToken::F64(v) => Number::F64(v),
    };

    AsonNode::Number(number)
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use pretty_assertions::assert_eq;

    use crate::{
        ast::{parser::parse_from, KeyValuePair, Number, Variant},
        error::Error,
        location::Position,
    };

    use super::AsonNode;

    #[test]
    fn test_parse_simple_value() {
        assert_eq!(
            parse_from(
                r#"
            123
            "#
            )
            .unwrap(),
            AsonNode::Number(Number::I32(123))
        );

        assert_eq!(
            parse_from(
                r#"
            true
            "#
            )
            .unwrap(),
            AsonNode::Boolean(true)
        );

        assert_eq!(
            parse_from(
                r#"
            '🍒'
            "#
            )
            .unwrap(),
            AsonNode::Char('🍒')
        );

        assert_eq!(
            parse_from(
                r#"
            "hello"
            "#
            )
            .unwrap(),
            AsonNode::String_("hello".to_owned())
        );

        assert_eq!(
            parse_from(
                r#"
            d"2024-03-17 10:01:11+08:00"
            "#
            )
            .unwrap(),
            AsonNode::DateTime(DateTime::parse_from_rfc3339("2024-03-17 10:01:11+08:00").unwrap())
        );
    }

    #[test]
    fn test_parse_byte_data() {
        assert_eq!(
            parse_from(
                r#"
            h"11 13 17 19"
            "#
            )
            .unwrap(),
            AsonNode::ByteData(vec![0x11u8, 0x13, 0x17, 0x19])
        );
    }

    #[test]
    fn test_parse_object() {
        let expect_object1 = AsonNode::Object(vec![
            KeyValuePair {
                key: "id".to_owned(),
                value: Box::new(AsonNode::Number(Number::I32(123))),
            },
            KeyValuePair {
                key: "name".to_owned(),
                value: Box::new(AsonNode::String_("foo".to_owned())),
            },
        ]);

        assert_eq!(
            parse_from(
                r#"
            {id:123,name:"foo"}
            "#
            )
            .unwrap(),
            expect_object1
        );

        assert_eq!(
            parse_from(
                r#"
            {
                id:123
                name:"foo"
            }
            "#
            )
            .unwrap(),
            expect_object1
        );

        assert_eq!(
            parse_from(
                r#"
            {
                id:123,
                name:"foo"
            }
            "#
            )
            .unwrap(),
            expect_object1
        );

        assert_eq!(
            parse_from(
                r#"
            {
                id: 123,
                name: "foo",
            }
            "#
            )
            .unwrap(),
            expect_object1
        );

        assert_eq!(
            parse_from(
                r#"
            {
                id: 123
                addr: Option::Some({
                    city: "ShenZhen"
                    street: Option::None
                })
            }
            "#
            )
            .unwrap(),
            AsonNode::Object(vec![
                KeyValuePair {
                    key: "id".to_owned(),
                    value: Box::new(AsonNode::Number(Number::I32(123))),
                },
                KeyValuePair {
                    key: "addr".to_owned(),
                    value: Box::new(AsonNode::Variant(Variant::with_value(
                        "Option",
                        "Some",
                        AsonNode::Object(vec![
                            KeyValuePair {
                                key: "city".to_owned(),
                                value: Box::new(AsonNode::String_("ShenZhen".to_owned())),
                            },
                            KeyValuePair {
                                key: "street".to_owned(),
                                value: Box::new(AsonNode::Variant(Variant::new("Option", "None"))),
                            },
                        ])
                    ))),
                },
            ])
        );

        // err: incorrect key name (should be without quote)
        assert!(matches!(
            parse_from(
                r#"{
    "id": 123,
    "name": "foo",
}"#
            ),
            Err(Error::MessageWithPosition(
                _,
                Position {
                    unit: 0,
                    index: 6,
                    line: 1,
                    column: 4
                }
            ))
        ));

        // err: missing key name
        assert!(matches!(
            parse_from(r#"{123}"#),
            Err(Error::MessageWithPosition(
                _,
                Position {
                    unit: 0,
                    index: 1,
                    line: 0,
                    column: 1
                }
            ))
        ));

        // err: missing '}'
        assert!(matches!(
            parse_from(r#"{id:123"#),
            Err(Error::MessageWithPosition(
                _,
                Position {
                    unit: 0,
                    index: 7,
                    line: 0,
                    column: 7
                }
            ))
        ));
    }

    #[test]
    fn test_parse_list() {
        let expect_list1 = AsonNode::List(vec![
            AsonNode::Number(Number::I32(123)),
            AsonNode::Number(Number::I32(456)),
            AsonNode::Number(Number::I32(789)),
        ]);

        assert_eq!(
            parse_from(
                r#"
            [123,456,789]
            "#
            )
            .unwrap(),
            expect_list1
        );

        assert_eq!(
            parse_from(
                r#"
            [
                123
                456
                789
            ]
            "#
            )
            .unwrap(),
            expect_list1
        );

        assert_eq!(
            parse_from(
                r#"
            [
                123,
                456,
                789
            ]
            "#
            )
            .unwrap(),
            expect_list1
        );

        assert_eq!(
            parse_from(
                r#"
            [
                123,
                456,
                789,
            ]
            "#
            )
            .unwrap(),
            expect_list1
        );

        // err: missing ']'
        assert!(matches!(parse_from(r#"[123,"#), Err(Error::Message(_))));
    }

    #[test]
    fn test_parse_tuple() {
        let expect_tuple1 = AsonNode::Tuple(vec![
            AsonNode::Number(Number::I32(123)),
            AsonNode::String_("foo".to_owned()),
            AsonNode::Boolean(true),
        ]);

        assert_eq!(
            parse_from(
                r#"
            (123,"foo",true)
            "#
            )
            .unwrap(),
            expect_tuple1
        );

        assert_eq!(
            parse_from(
                r#"
            (
                123
                "foo"
                true
            )
            "#
            )
            .unwrap(),
            expect_tuple1
        );

        assert_eq!(
            parse_from(
                r#"
            (
                123,
                "foo",
                true
            )
            "#
            )
            .unwrap(),
            expect_tuple1
        );

        assert_eq!(
            parse_from(
                r#"
            (
                123,
                "foo",
                true,
            )
            "#
            )
            .unwrap(),
            expect_tuple1
        );

        // err: empty tuple
        assert!(matches!(
            parse_from(r#"()"#),
            Err(Error::MessageWithPosition(
                _,
                Position {
                    unit: 0,
                    index: 1,
                    line: 0,
                    column: 1
                }
            ))
        ));

        // err: missing ')'
        assert!(matches!(parse_from(r#"(123,"#), Err(Error::Message(_))));
    }

    #[test]
    fn test_parse_variant() {
        // empty value
        assert_eq!(
            parse_from(
                r#"
            Option::None
            "#
            )
            .unwrap(),
            AsonNode::Variant(Variant::new("Option", "None"))
        );

        // single value
        assert_eq!(
            parse_from(
                r#"
            Option::Some(123)
            "#
            )
            .unwrap(),
            AsonNode::Variant(Variant::with_value(
                "Option",
                "Some",
                AsonNode::Number(Number::I32(123))
            ))
        );

        // multiple values
        assert_eq!(
            parse_from(
                r#"
            Color::RGB(100,75,0)
            "#
            )
            .unwrap(),
            AsonNode::Variant(Variant::with_values(
                "Color",
                "RGB",
                vec![
                    AsonNode::Number(Number::I32(100)),
                    AsonNode::Number(Number::I32(75)),
                    AsonNode::Number(Number::I32(0)),
                ]
            ))
        );

        // object value
        assert_eq!(
            parse_from(
                r#"
            Shape::Rect{width:123, height:456}
            "#
            )
            .unwrap(),
            AsonNode::Variant(Variant::with_object(
                "Shape",
                "Rect",
                vec![
                    KeyValuePair::new("width", AsonNode::Number(Number::I32(123))),
                    KeyValuePair::new("height", AsonNode::Number(Number::I32(456))),
                ]
            ))
        );

        // err: missing value(s)
        assert!(matches!(
            parse_from(r#"Option::Some()"#),
            Err(Error::MessageWithPosition(
                _,
                Position {
                    unit: 0,
                    index: 13,
                    line: 0,
                    column: 13
                }
            ))
        ));

        // err: missing ')'
        assert!(matches!(
            parse_from(r#"Color::RGB(11,13"#),
            Err(Error::Message(_))
        ));

        // err: missing '}'
        assert!(matches!(
            parse_from(r#"Color::Rect{width:11"#),
            Err(Error::MessageWithPosition(
                _,
                Position {
                    unit: 0,
                    index: 20,
                    line: 0,
                    column: 20
                }
            ))
        ));
    }

    #[test]
    fn test_parse_complex() {
        assert_eq!(
            parse_from(
                r#"
            {
                id:123
                name:"hello"
                orders: [
                    (1, "foo", true)
                    (2, "bar", false)
                ]
                group: {
                    active: true
                    permissions:[
                        {number:11, title: "read"}
                        {number:13, title: "write"}
                    ]
                }
            }
            "#
            )
            .unwrap(),
            AsonNode::Object(vec![
                KeyValuePair {
                    key: "id".to_owned(),
                    value: Box::new(AsonNode::Number(Number::I32(123))),
                },
                KeyValuePair {
                    key: "name".to_owned(),
                    value: Box::new(AsonNode::String_("hello".to_owned())),
                },
                KeyValuePair {
                    key: "orders".to_owned(),
                    value: Box::new(AsonNode::List(vec![
                        AsonNode::Tuple(vec![
                            AsonNode::Number(Number::I32(1)),
                            AsonNode::String_("foo".to_owned()),
                            AsonNode::Boolean(true),
                        ]),
                        AsonNode::Tuple(vec![
                            AsonNode::Number(Number::I32(2)),
                            AsonNode::String_("bar".to_owned()),
                            AsonNode::Boolean(false),
                        ]),
                    ])),
                },
                KeyValuePair {
                    key: "group".to_owned(),
                    value: Box::new(AsonNode::Object(vec![
                        KeyValuePair {
                            key: "active".to_owned(),
                            value: Box::new(AsonNode::Boolean(true)),
                        },
                        KeyValuePair {
                            key: "permissions".to_owned(),
                            value: Box::new(AsonNode::List(vec![
                                AsonNode::Object(vec![
                                    KeyValuePair {
                                        key: "number".to_owned(),
                                        value: Box::new(AsonNode::Number(Number::I32(11))),
                                    },
                                    KeyValuePair {
                                        key: "title".to_owned(),
                                        value: Box::new(AsonNode::String_("read".to_owned())),
                                    },
                                ]),
                                AsonNode::Object(vec![
                                    KeyValuePair {
                                        key: "number".to_owned(),
                                        value: Box::new(AsonNode::Number(Number::I32(13))),
                                    },
                                    KeyValuePair {
                                        key: "title".to_owned(),
                                        value: Box::new(AsonNode::String_("write".to_owned())),
                                    },
                                ]),
                            ])),
                        },
                    ])),
                },
            ])
        );

        // err: document does not end properly
        assert!(matches!(
            parse_from(r#"true false"#),
            Err(Error::MessageWithPosition(
                _,
                Position {
                    unit: 0,
                    index: 5,
                    line: 0,
                    column: 5
                }
            ))
        ));
    }
}
