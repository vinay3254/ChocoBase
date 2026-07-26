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
            Token::Insert => self.parse_insert()?,
            Token::Select => self.parse_select()?,
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

    fn parse_primary_expr(&mut self) -> Result<Expr, ParseError> {
        let offset = self.peek_offset();
        match self.advance() {
            Token::IntLiteral(n) => Ok(Expr::IntLiteral(n)),
            Token::StringLiteral(s) => Ok(Expr::StringLiteral(s)),
            Token::True => Ok(Expr::BoolLiteral(true)),
            Token::False => Ok(Expr::BoolLiteral(false)),
            Token::Null => Ok(Expr::Null),
            Token::Identifier(name) => Ok(Expr::Column(name)),
            Token::LParen => {
                let e = self.parse_where_expr()?;
                self.expect(&Token::RParen)?;
                Ok(e)
            }
            other => Err(ParseError::Syntax { offset, message: format!("expected an expression, found {other:?}") }),
        }
    }

    fn parse_insert(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Insert)?;
        self.expect(&Token::Into)?;
        let table = self.expect_identifier()?;

        let columns = if matches!(self.peek(), Token::LParen) {
            self.advance();
            let mut cols = Vec::new();
            loop {
                cols.push(self.expect_identifier()?);
                match self.peek() {
                    Token::Comma => { self.advance(); }
                    Token::RParen => break,
                    _ => return Err(ParseError::Syntax { offset: self.peek_offset(), message: "expected ',' or ')'".into() }),
                }
            }
            self.expect(&Token::RParen)?;
            Some(cols)
        } else {
            None
        };

        self.expect(&Token::Values)?;
        let mut rows = Vec::new();
        loop {
            self.expect(&Token::LParen)?;
            let mut row = Vec::new();
            loop {
                row.push(self.parse_primary_expr()?);
                match self.peek() {
                    Token::Comma => { self.advance(); }
                    Token::RParen => break,
                    _ => return Err(ParseError::Syntax { offset: self.peek_offset(), message: "expected ',' or ')'".into() }),
                }
            }
            self.expect(&Token::RParen)?;
            rows.push(row);
            match self.peek() {
                Token::Comma => { self.advance(); }
                _ => break,
            }
        }
        Ok(Statement::Insert { table, columns, rows })
    }

    fn parse_where_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Token::Or) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BinaryOp { op: BinOp::Or, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_comparison()?;
        while matches!(self.peek(), Token::And) {
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::BinaryOp { op: BinOp::And, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr, ParseError> {
        let left = self.parse_primary_expr()?;
        match self.peek() {
            Token::Eq | Token::NotEq | Token::Lt | Token::LtEq | Token::Gt | Token::GtEq => {
                let op = match self.advance() {
                    Token::Eq => BinOp::Eq,
                    Token::NotEq => BinOp::NotEq,
                    Token::Lt => BinOp::Lt,
                    Token::LtEq => BinOp::LtEq,
                    Token::Gt => BinOp::Gt,
                    Token::GtEq => BinOp::GtEq,
                    _ => unreachable!(),
                };
                let right = self.parse_primary_expr()?;
                Ok(Expr::BinaryOp { op, left: Box::new(left), right: Box::new(right) })
            }
            Token::Is => {
                self.advance();
                let negated = if matches!(self.peek(), Token::Not) {
                    self.advance();
                    true
                } else {
                    false
                };
                self.expect(&Token::Null)?;
                Ok(Expr::IsNull { expr: Box::new(left), negated })
            }
            _ => Ok(left),
        }
    }

    fn parse_select(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Select)?;
        let columns = if matches!(self.peek(), Token::Star) {
            self.advance();
            SelectColumns::All
        } else {
            let mut cols = vec![self.expect_identifier()?];
            while matches!(self.peek(), Token::Comma) {
                self.advance();
                cols.push(self.expect_identifier()?);
            }
            SelectColumns::List(cols)
        };
        self.expect(&Token::From)?;
        let table = self.expect_identifier()?;

        let where_clause = if matches!(self.peek(), Token::Where) {
            self.advance();
            Some(self.parse_where_expr()?)
        } else {
            None
        };

        let order_by = if matches!(self.peek(), Token::Order) {
            self.advance();
            self.expect(&Token::By)?;
            let col = self.expect_identifier()?;
            let desc = match self.peek() {
                Token::Desc => { self.advance(); true }
                Token::Asc => { self.advance(); false }
                _ => false,
            };
            Some((col, desc))
        } else {
            None
        };

        let limit = if matches!(self.peek(), Token::Limit) {
            self.advance();
            let offset = self.peek_offset();
            match self.advance() {
                Token::IntLiteral(n) => Some(n),
                other => return Err(ParseError::Syntax { offset, message: format!("expected integer after LIMIT, found {other:?}") }),
            }
        } else {
            None
        };

        Ok(Statement::Select { columns, table, where_clause, order_by, limit })
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

    #[test]
    fn parses_insert_with_explicit_columns() {
        let stmt = parse("INSERT INTO users (id, name) VALUES (1, 'Ada')").unwrap();
        assert_eq!(
            stmt,
            Statement::Insert {
                table: "users".into(),
                columns: Some(vec!["id".into(), "name".into()]),
                rows: vec![vec![Expr::IntLiteral(1), Expr::StringLiteral("Ada".into())]],
            }
        );
    }

    #[test]
    fn parses_insert_multiple_rows_without_columns() {
        let stmt = parse("INSERT INTO t VALUES (1, NULL), (2, TRUE)").unwrap();
        assert_eq!(
            stmt,
            Statement::Insert {
                table: "t".into(),
                columns: None,
                rows: vec![
                    vec![Expr::IntLiteral(1), Expr::Null],
                    vec![Expr::IntLiteral(2), Expr::BoolLiteral(true)],
                ],
            }
        );
    }

    #[test]
    fn parses_select_star_with_where_and_precedence() {
        // Confirms AND binds tighter than OR: `a OR b AND c` parses as `a OR (b AND c)`.
        let stmt = parse("SELECT * FROM t WHERE a = 1 OR b = 2 AND c = 3").unwrap();
        match stmt {
            Statement::Select { where_clause: Some(Expr::BinaryOp { op: BinOp::Or, left, right }), .. } => {
                assert_eq!(*left, Expr::BinaryOp {
                    op: BinOp::Eq,
                    left: Box::new(Expr::Column("a".into())),
                    right: Box::new(Expr::IntLiteral(1)),
                });
                assert!(matches!(*right, Expr::BinaryOp { op: BinOp::And, .. }));
            }
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn parses_select_with_is_null() {
        let stmt = parse("SELECT id FROM t WHERE name IS NOT NULL").unwrap();
        assert_eq!(
            stmt,
            Statement::Select {
                columns: SelectColumns::List(vec!["id".into()]),
                table: "t".into(),
                where_clause: Some(Expr::IsNull { expr: Box::new(Expr::Column("name".into())), negated: true }),
                order_by: None,
                limit: None,
            }
        );
    }

    #[test]
    fn parses_select_with_order_by_and_limit() {
        let stmt = parse("SELECT * FROM t ORDER BY id DESC LIMIT 10").unwrap();
        match stmt {
            Statement::Select { order_by, limit, .. } => {
                assert_eq!(order_by, Some(("id".into(), true)));
                assert_eq!(limit, Some(10));
            }
            other => panic!("unexpected parse result: {other:?}"),
        }
    }
}
