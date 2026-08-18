use crate::error::ParseError;
use crate::sql::token::{SpannedToken, Token};

pub fn tokenize(src: &str) -> Result<Vec<SpannedToken>, ParseError> {
    // Iterate over decoded `char`s (via `char_indices`), not raw bytes: casting an
    // arbitrary byte to `char` (the previous approach) misinterprets UTF-8
    // continuation bytes as Latin-1 code points, some of which look "alphabetic" to
    // Rust's char classification -- corrupting string-literal content and risking a
    // "not a char boundary" panic on any non-ASCII input (e.g. a TEXT literal like
    // 'café'). `char_indices()` still reports byte offsets, so `SpannedToken.offset`
    // is unaffected.
    let mut chars = src.char_indices().peekable();
    let mut tokens = Vec::new();

    while let Some(&(start, c)) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        match c {
            '(' => { chars.next(); tokens.push(SpannedToken { token: Token::LParen, offset: start }); }
            ')' => { chars.next(); tokens.push(SpannedToken { token: Token::RParen, offset: start }); }
            ',' => { chars.next(); tokens.push(SpannedToken { token: Token::Comma, offset: start }); }
            '*' => { chars.next(); tokens.push(SpannedToken { token: Token::Star, offset: start }); }
            ';' => { chars.next(); tokens.push(SpannedToken { token: Token::Semicolon, offset: start }); }
            '=' => { chars.next(); tokens.push(SpannedToken { token: Token::Eq, offset: start }); }
            '<' => {
                chars.next();
                match chars.peek() {
                    Some(&(_, '=')) => {
                        chars.next();
                        tokens.push(SpannedToken { token: Token::LtEq, offset: start });
                    }
                    Some(&(_, '>')) => {
                        chars.next();
                        tokens.push(SpannedToken { token: Token::NotEq, offset: start });
                    }
                    _ => tokens.push(SpannedToken { token: Token::Lt, offset: start }),
                }
            }
            '>' => {
                chars.next();
                match chars.peek() {
                    Some(&(_, '=')) => {
                        chars.next();
                        tokens.push(SpannedToken { token: Token::GtEq, offset: start });
                    }
                    _ => tokens.push(SpannedToken { token: Token::Gt, offset: start }),
                }
            }
            '!' => {
                chars.next();
                match chars.peek() {
                    Some(&(_, '=')) => {
                        chars.next();
                        tokens.push(SpannedToken { token: Token::NotEq, offset: start });
                    }
                    _ => return Err(ParseError::Syntax { offset: start, message: "unexpected '!'".into() }),
                }
            }
            '\'' => {
                chars.next();
                let mut s = String::new();
                loop {
                    match chars.next() {
                        None => {
                            return Err(ParseError::Syntax {
                                offset: start,
                                message: "unterminated string literal".into(),
                            })
                        }
                        Some((_, '\'')) => {
                            if let Some(&(_, '\'')) = chars.peek() {
                                chars.next();
                                s.push('\'');
                            } else {
                                break;
                            }
                        }
                        Some((_, ch)) => s.push(ch),
                    }
                }
                tokens.push(SpannedToken { token: Token::StringLiteral(s), offset: start });
            }
            c if c.is_ascii_digit() => {
                let mut end = start + c.len_utf8();
                chars.next();
                while let Some(&(p, c2)) = chars.peek() {
                    if c2.is_ascii_digit() {
                        end = p + c2.len_utf8();
                        chars.next();
                    } else {
                        break;
                    }
                }
                let text = &src[start..end];
                let n: i64 = text
                    .parse()
                    .map_err(|_| ParseError::Syntax { offset: start, message: "invalid integer literal".into() })?;
                tokens.push(SpannedToken { token: Token::IntLiteral(n), offset: start });
            }
            c if c.is_alphabetic() || c == '_' => {
                let mut end = start + c.len_utf8();
                chars.next();
                while let Some(&(p, c2)) = chars.peek() {
                    if c2.is_alphanumeric() || c2 == '_' {
                        end = p + c2.len_utf8();
                        chars.next();
                    } else {
                        break;
                    }
                }
                let text = &src[start..end];
                tokens.push(SpannedToken { token: keyword_or_identifier(text), offset: start });
            }
            _ => return Err(ParseError::Syntax { offset: start, message: format!("unexpected character '{c}'") }),
        }
    }
    tokens.push(SpannedToken { token: Token::Eof, offset: src.len() });
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
        "BEGIN" => Token::Begin,
        "COMMIT" => Token::Commit,
        "ROLLBACK" => Token::Rollback,
        "TRANSACTION" => Token::Transaction,
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

    #[test]
    fn tokenizes_non_ascii_utf8_string_literal_without_corruption_or_panic() {
        // TEXT values are UTF-8 (per the design's row/key encoding), so a string
        // literal containing multi-byte characters must round-trip exactly rather
        // than being corrupted or causing a byte-index panic. `é` and `€` are 2-
        // and 3-byte UTF-8 sequences respectively.
        assert_eq!(kinds("'café €5'"), vec![Token::StringLiteral("café €5".into()), Token::Eof]);
    }

    #[test]
    fn tokenizes_non_ascii_identifier_and_reports_correct_offsets() {
        // A non-ASCII identifier must not desynchronize byte offsets for tokens
        // that follow it -- offsets are byte positions (matching how the rest of
        // the engine slices &str), not char counts.
        let tokens = tokenize("café = 1").unwrap();
        let offsets: Vec<usize> = tokens.iter().map(|t| t.offset).collect();
        assert_eq!(
            tokens.iter().map(|t| t.token.clone()).collect::<Vec<_>>(),
            vec![Token::Identifier("café".into()), Token::Eq, Token::IntLiteral(1), Token::Eof]
        );
        // "café" is 5 bytes (c=1,a=1,f=1,é=2), so '=' starts at byte offset 6 (after
        // the trailing space), not char-index 5.
        assert_eq!(offsets, vec![0, 6, 8, 9]);
    }
}
