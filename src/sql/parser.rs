use crate::error::ParseError;
use crate::sql::ast::*;
use crate::sql::lexer::tokenize;
use crate::sql::token::{SpannedToken, Token};
use crate::types::value::ColumnType;

pub struct Parser {
    tokens: Vec<SpannedToken>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<SpannedToken>) -> Self {
        Parser { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos].token
    }

    fn peek_offset(&self) -> usize {
        self.tokens[self.pos].offset
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].token.clone();
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, expected: &Token) -> Result<(), ParseError> {
        if std::mem::discriminant(self.peek()) == std::mem::discriminant(expected) {
            self.advance();
            Ok(())
        } else {
            Err(ParseError::Syntax {
                offset: self.peek_offset(),
                message: format!("expected {expected:?}, found {:?}", self.peek()),
            })
        }
    }

    fn expect_identifier(&mut self) -> Result<String, ParseError> {
        let offset = self.peek_offset();
        match self.advance() {
            Token::Identifier(s) => Ok(s),
            other => Err(ParseError::Syntax { offset, message: format!("expected identifier, found {other:?}") }),
        }
    }

    pub fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        let stmt = match self.peek() {
            Token::Create => self.parse_create()?,
            Token::Drop => self.parse_drop()?,
            _ => return Err(ParseError::Syntax { offset: self.peek_offset(), message: "expected a statement".into() }),
        };
        if matches!(self.peek(), Token::Semicolon) {
            self.advance();
        }
        Ok(stmt)
    }

    fn parse_create(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Create)?;
        match self.peek() {
            Token::Table => self.parse_create_table(),
            Token::Index => Err(ParseError::Syntax { offset: self.peek_offset(), message: "CREATE INDEX not yet implemented".into() }),
            _ => Err(ParseError::Syntax { offset: self.peek_offset(), message: "expected TABLE or INDEX after CREATE".into() }),
        }
    }

    fn parse_create_table(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Table)?;
        let name = self.expect_identifier()?;
        self.expect(&Token::LParen)?;
        let mut columns = Vec::new();
        loop {
            let cname = self.expect_identifier()?;
            let ty_offset = self.peek_offset();
            let ty = match self.advance() {
                Token::KwInteger => ColumnType::Integer,
                Token::KwText => ColumnType::Text,
                Token::KwBoolean => ColumnType::Boolean,
                other => return Err(ParseError::Syntax { offset: ty_offset, message: format!("expected a type, found {other:?}") }),
            };
            let mut not_null = false;
            let mut primary_key = false;
            loop {
                match self.peek() {
                    Token::Not => { self.advance(); self.expect(&Token::Null)?; not_null = true; }
                    Token::Primary => { self.advance(); self.expect(&Token::Key)?; primary_key = true; }
                    _ => break,
                }
            }
            columns.push(ColumnDef { name: cname, ty, not_null, primary_key });
            match self.peek() {
                Token::Comma => { self.advance(); }
                Token::RParen => break,
                _ => return Err(ParseError::Syntax { offset: self.peek_offset(), message: "expected ',' or ')'".into() }),
            }
        }
        self.expect(&Token::RParen)?;
        Ok(Statement::CreateTable { name, columns })
    }

    fn parse_drop(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Drop)?;
        let offset = self.peek_offset();
        match self.advance() {
            Token::Table => Ok(Statement::DropTable { name: self.expect_identifier()? }),
            Token::Index => Err(ParseError::Syntax { offset, message: "DROP INDEX not yet implemented".into() }),
            other => Err(ParseError::Syntax { offset, message: format!("expected TABLE or INDEX, found {other:?}") }),
        }
    }
}

pub fn parse(sql: &str) -> Result<Statement, ParseError> {
    let tokens = tokenize(sql)?;
    let mut parser = Parser::new(tokens);
    parser.parse_statement()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_create_table() {
        let stmt = parse("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)").unwrap();
        assert_eq!(
            stmt,
            Statement::CreateTable {
                name: "users".into(),
                columns: vec![
                    ColumnDef { name: "id".into(), ty: ColumnType::Integer, not_null: false, primary_key: true },
                    ColumnDef { name: "name".into(), ty: ColumnType::Text, not_null: true, primary_key: false },
                ],
            }
        );
    }

    #[test]
    fn parses_drop_table() {
        assert_eq!(parse("DROP TABLE users").unwrap(), Statement::DropTable { name: "users".into() });
    }

    #[test]
    fn reports_syntax_error_with_offset() {
        let err = parse("CREATE TABLE").unwrap_err();
        assert!(matches!(err, ParseError::Syntax { .. }));
    }
}
