use crate::error::ParseError;
use crate::sql::token::{SpannedToken, Token};

pub fn tokenize(src: &str) -> Result<Vec<SpannedToken>, ParseError> {
    let bytes = src.as_bytes();
    let mut pos = 0;
    let mut tokens = Vec::new();

    while pos < bytes.len() {
        let c = bytes[pos] as char;
        if c.is_whitespace() {
            pos += 1;
            continue;
        }
        let start = pos;
        match c {
            '(' => { tokens.push(SpannedToken { token: Token::LParen, offset: start }); pos += 1; }
            ')' => { tokens.push(SpannedToken { token: Token::RParen, offset: start }); pos += 1; }
            ',' => { tokens.push(SpannedToken { token: Token::Comma, offset: start }); pos += 1; }
            '*' => { tokens.push(SpannedToken { token: Token::Star, offset: start }); pos += 1; }
            ';' => { tokens.push(SpannedToken { token: Token::Semicolon, offset: start }); pos += 1; }
            '=' => { tokens.push(SpannedToken { token: Token::Eq, offset: start }); pos += 1; }
            '<' => {
                pos += 1;
                if pos < bytes.len() && bytes[pos] as char == '=' {
                    tokens.push(SpannedToken { token: Token::LtEq, offset: start });
                    pos += 1;
                } else if pos < bytes.len() && bytes[pos] as char == '>' {
                    tokens.push(SpannedToken { token: Token::NotEq, offset: start });
                    pos += 1;
                } else {
                    tokens.push(SpannedToken { token: Token::Lt, offset: start });
                }
            }
            '>' => {
                pos += 1;
                if pos < bytes.len() && bytes[pos] as char == '=' {
                    tokens.push(SpannedToken { token: Token::GtEq, offset: start });
                    pos += 1;
                } else {
                    tokens.push(SpannedToken { token: Token::Gt, offset: start });
                }
            }
            '!' => {
                pos += 1;
                if pos < bytes.len() && bytes[pos] as char == '=' {
                    tokens.push(SpannedToken { token: Token::NotEq, offset: start });
                    pos += 1;
                } else {
                    return Err(ParseError::Syntax { offset: start, message: "unexpected '!'".into() });
                }
            }
            '\'' => {
                pos += 1;
                let mut s = String::new();
                loop {
                    if pos >= bytes.len() {
                        return Err(ParseError::Syntax { offset: start, message: "unterminated string literal".into() });
                    }
                    let ch = bytes[pos] as char;
                    if ch == '\'' {
                        if pos + 1 < bytes.len() && bytes[pos + 1] as char == '\'' {
                            s.push('\'');
                            pos += 2;
                        } else {
                            pos += 1;
                            break;
                        }
                    } else {
                        s.push(ch);
                        pos += 1;
                    }
                }
                tokens.push(SpannedToken { token: Token::StringLiteral(s), offset: start });
            }
            c if c.is_ascii_digit() => {
                while pos < bytes.len() && (bytes[pos] as char).is_ascii_digit() {
                    pos += 1;
                }
                let text = &src[start..pos];
                let n: i64 = text
                    .parse()
                    .map_err(|_| ParseError::Syntax { offset: start, message: "invalid integer literal".into() })?;
                tokens.push(SpannedToken { token: Token::IntLiteral(n), offset: start });
            }
            c if c.is_alphabetic() || c == '_' => {
                while pos < bytes.len() && ((bytes[pos] as char).is_alphanumeric() || bytes[pos] as char == '_') {
                    pos += 1;
                }
                let text = &src[start..pos];
                tokens.push(SpannedToken { token: keyword_or_identifier(text), offset: start });
            }
            _ => return Err(ParseError::Syntax { offset: start, message: format!("unexpected character '{c}'") }),
        }
    }
    tokens.push(SpannedToken { token: Token::Eof, offset: bytes.len() });
    Ok(tokens)
}

fn keyword_or_identifier(text: &str) -> Token {
    match text.to_uppercase().as_str() {
        "CREATE" => Token::Create,
        "TABLE" => Token::Table,
        "DROP" => Token::Drop,
        "INDEX" => Token::Index,
        "ON" => Token::On,
        "INSERT" => Token::Insert,
        "INTO" => Token::Into,
        "VALUES" => Token::Values,
        "SELECT" => Token::Select,
        "FROM" => Token::From,
        "WHERE" => Token::Where,
        "UPDATE" => Token::Update,
        "SET" => Token::Set,
        "DELETE" => Token::Delete,
        "ORDER" => Token::Order,
        "BY" => Token::By,
        "ASC" => Token::Asc,
        "DESC" => Token::Desc,
        "LIMIT" => Token::Limit,
        "NOT" => Token::Not,
        "NULL" => Token::Null,
        "PRIMARY" => Token::Primary,
        "KEY" => Token::Key,
        "AND" => Token::And,
        "OR" => Token::Or,
        "IS" => Token::Is,
        "INTEGER" => Token::KwInteger,
        "TEXT" => Token::KwText,
        "BOOLEAN" => Token::KwBoolean,
        "TRUE" => Token::True,
        "FALSE" => Token::False,
        _ => Token::Identifier(text.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Token> {
        tokenize(src).unwrap().into_iter().map(|t| t.token).collect()
    }

    #[test]
    fn tokenizes_create_table() {
        let tokens = kinds("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)");
        assert_eq!(
            tokens,
            vec![
                Token::Create, Token::Table, Token::Identifier("users".into()), Token::LParen,
                Token::Identifier("id".into()), Token::KwInteger, Token::Primary, Token::Key, Token::Comma,
                Token::Identifier("name".into()), Token::KwText, Token::Not, Token::Null,
                Token::RParen, Token::Eof,
            ]
        );
    }

    #[test]
    fn tokenizes_operators() {
        assert_eq!(kinds("<= >= <> != < > ="), vec![
            Token::LtEq, Token::GtEq, Token::NotEq, Token::NotEq, Token::Lt, Token::Gt, Token::Eq, Token::Eof
        ]);
    }

    #[test]
    fn tokenizes_string_literal_with_escaped_quote() {
        assert_eq!(kinds("'it''s'"), vec![Token::StringLiteral("it's".into()), Token::Eof]);
    }

    #[test]
    fn tokenizes_keywords_case_insensitively() {
        assert_eq!(kinds("select FROM Where"), vec![Token::Select, Token::From, Token::Where, Token::Eof]);
    }

    #[test]
    fn reports_offset_on_unterminated_string() {
        let err = tokenize("'abc").unwrap_err();
        match err {
            ParseError::Syntax { offset, .. } => assert_eq!(offset, 0),
        }
    }
}
