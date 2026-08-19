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
            Token::Role => Ok("role".into()),
            Token::User => Ok("user".into()),
            Token::Password => Ok("password".into()),
            Token::Policy => Ok("policy".into()),
            Token::Enable => Ok("enable".into()),
            Token::Disable => Ok("disable".into()),
            Token::Count => Ok("count".into()),
            Token::Sum => Ok("sum".into()),
            Token::Avg => Ok("avg".into()),
            Token::Min => Ok("min".into()),
            Token::Max => Ok("max".into()),
            Token::KwInteger => Ok("integer".into()),
            Token::KwText => Ok("text".into()),
            Token::KwBoolean => Ok("boolean".into()),
            Token::KwJson => Ok("json".into()),
            other => Err(ParseError::Syntax {
                offset,
                message: format!("expected identifier, found {other:?}"),
            }),
        }
    }

    pub fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        let stmt = match self.peek() {
            Token::Explain => {
                self.advance();
                let inner = self.parse_statement()?;
                Statement::Explain(Box::new(inner))
            }
            Token::Create => self.parse_create()?,
            Token::Drop => self.parse_drop()?,
            Token::Alter => self.parse_alter()?,
            Token::Insert => self.parse_insert()?,
            Token::Select => self.parse_select()?,
            Token::Update => self.parse_update()?,
            Token::Delete => self.parse_delete()?,
            Token::Begin => {
                self.advance();
                if matches!(self.peek(), Token::Transaction) {
                    self.advance();
                }
                Statement::Begin
            }
            Token::Commit => {
                self.advance();
                if matches!(self.peek(), Token::Transaction) {
                    self.advance();
                }
                Statement::Commit
            }
            Token::Rollback => {
                self.advance();
                if matches!(self.peek(), Token::Transaction) {
                    self.advance();
                }
                Statement::Rollback
            }
            _ => {
                return Err(ParseError::Syntax {
                    offset: self.peek_offset(),
                    message: "expected a statement".into(),
                })
            }
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
            Token::Index => self.parse_create_index(),
            Token::User => self.parse_create_user(),
            Token::Policy => self.parse_create_policy(),
            _ => Err(ParseError::Syntax {
                offset: self.peek_offset(),
                message: "expected TABLE, INDEX, USER, or POLICY after CREATE".into(),
            }),
        }
    }

    fn parse_create_user(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::User)?;
        let username = self.expect_identifier()?;
        if matches!(self.peek(), Token::With) {
            self.advance();
        }
        self.expect(&Token::Password)?;
        let pwd_offset = self.peek_offset();
        let password = match self.advance() {
            Token::StringLiteral(s) => s,
            other => {
                return Err(ParseError::Syntax {
                    offset: pwd_offset,
                    message: format!("expected password string literal, found {other:?}"),
                })
            }
        };
        let mut role = None;
        if matches!(self.peek(), Token::Role) {
            self.advance();
            let role_offset = self.peek_offset();
            role = match self.advance() {
                Token::StringLiteral(s) => Some(s),
                Token::Identifier(s) => Some(s),
                other => {
                    return Err(ParseError::Syntax {
                        offset: role_offset,
                        message: format!("expected role string or identifier, found {other:?}"),
                    })
                }
            };
        }
        Ok(Statement::CreateUser {
            username,
            password,
            role,
        })
    }

    fn parse_create_policy(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Policy)?;
        let name = self.expect_identifier()?;
        self.expect(&Token::On)?;
        let table = self.expect_identifier()?;
        self.expect(&Token::For)?;
        let cmd = match self.peek() {
            Token::Select => {
                self.advance();
                crate::types::schema::PolicyCmd::Select
            }
            Token::Insert => {
                self.advance();
                crate::types::schema::PolicyCmd::Insert
            }
            Token::Update => {
                self.advance();
                crate::types::schema::PolicyCmd::Update
            }
            Token::Delete => {
                self.advance();
                crate::types::schema::PolicyCmd::Delete
            }
            Token::All => {
                self.advance();
                crate::types::schema::PolicyCmd::All
            }
            other => {
                return Err(ParseError::Syntax {
                    offset: self.peek_offset(),
                    message: format!(
                    "expected SELECT, INSERT, UPDATE, DELETE, or ALL after FOR, found {other:?}"
                ),
                })
            }
        };
        let mut using_expr = None;
        if matches!(self.peek(), Token::Using) {
            self.advance();
            self.expect(&Token::LParen)?;
            using_expr = Some(self.parse_where_expr()?);
            self.expect(&Token::RParen)?;
        }
        let mut with_check = None;
        if matches!(self.peek(), Token::With) || matches!(self.peek(), Token::Check) {
            if matches!(self.peek(), Token::With) {
                self.advance();
            }
            self.expect(&Token::Check)?;
            self.expect(&Token::LParen)?;
            with_check = Some(self.parse_where_expr()?);
            self.expect(&Token::RParen)?;
        }
        Ok(Statement::CreatePolicy {
            name,
            table,
            cmd,
            using_expr,
            with_check,
        })
    }

    fn parse_alter(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Alter)?;
        self.expect(&Token::Table)?;
        let table = self.expect_identifier()?;
        match self.advance() {
            Token::Enable => {
                self.expect(&Token::Row)?;
                self.expect(&Token::Level)?;
                self.expect(&Token::Security)?;
                Ok(Statement::AlterTableRls {
                    table,
                    enabled: true,
                })
            }
            Token::Disable => {
                self.expect(&Token::Row)?;
                self.expect(&Token::Level)?;
                self.expect(&Token::Security)?;
                Ok(Statement::AlterTableRls {
                    table,
                    enabled: false,
                })
            }
            Token::Add => {
                if matches!(self.peek(), Token::Column) {
                    self.advance();
                }
                let cname = self.expect_identifier()?;
                let ty_offset = self.peek_offset();
                let ty = match self.advance() {
                    Token::KwInteger => ColumnType::Integer,
                    Token::KwFloat => ColumnType::Float,
                    Token::KwText => ColumnType::Text,
                    Token::KwBoolean => ColumnType::Boolean,
                    Token::KwJson => ColumnType::Json,
                    Token::KwVector => {
                        let mut dim = 1536;
                        if matches!(self.peek(), Token::LParen) {
                            self.advance();
                            if let Token::IntLiteral(n) = self.advance() {
                                dim = n as usize;
                            }
                            self.expect(&Token::RParen)?;
                        }
                        ColumnType::Vector(dim)
                    }
                    other => {
                        return Err(ParseError::Syntax {
                            offset: ty_offset,
                            message: format!("expected column type, found {other:?}"),
                        });
                    }
                };

                let mut not_null = false;
                let mut primary_key = false;

                while matches!(self.peek(), Token::Not | Token::Primary) {
                    if matches!(self.peek(), Token::Not) {
                        self.advance();
                        self.expect(&Token::Null)?;
                        not_null = true;
                    } else if matches!(self.peek(), Token::Primary) {
                        self.advance();
                        self.expect(&Token::Key)?;
                        primary_key = true;
                    }
                }

                Ok(Statement::AlterTableAddColumn {
                    table,
                    column: crate::sql::ast::ColumnDef {
                        name: cname,
                        ty,
                        not_null,
                        primary_key,
                    },
                })
            }
            Token::Drop => {
                if matches!(self.peek(), Token::Column) {
                    self.advance();
                }
                let cname = self.expect_identifier()?;
                Ok(Statement::AlterTableDropColumn {
                    table,
                    column: cname,
                })
            }
            Token::Rename => {
                self.expect(&Token::To)?;
                let new_name = self.expect_identifier()?;
                Ok(Statement::AlterTableRename { table, new_name })
            }
            other => Err(ParseError::Syntax {
                offset: self.peek_offset(),
                message: format!(
                    "expected ENABLE, DISABLE, ADD, DROP, or RENAME after ALTER TABLE, found {other:?}"
                ),
            }),
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
                Token::KwFloat => ColumnType::Float,
                Token::KwText => ColumnType::Text,
                Token::KwBoolean => ColumnType::Boolean,
                Token::KwJson => ColumnType::Json,
                Token::KwVector => {
                    let mut dim = 1536;
                    if matches!(self.peek(), Token::LParen) {
                        self.advance();
                        if let Token::IntLiteral(n) = self.advance() {
                            dim = n as usize;
                        }
                        self.expect(&Token::RParen)?;
                    }
                    ColumnType::Vector(dim)
                }
                other => {
                    return Err(ParseError::Syntax {
                        offset: ty_offset,
                        message: format!("expected type, found {other:?}"),
                    })
                }
            };
            let mut not_null = false;
            let mut primary_key = false;
            while matches!(self.peek(), Token::Not | Token::Primary) {
                if matches!(self.peek(), Token::Not) {
                    self.advance();
                    self.expect(&Token::Null)?;
                    not_null = true;
                } else if matches!(self.peek(), Token::Primary) {
                    self.advance();
                    self.expect(&Token::Key)?;
                    primary_key = true;
                }
            }
            columns.push(ColumnDef {
                name: cname,
                ty,
                not_null,
                primary_key,
            });
            match self.peek() {
                Token::Comma => {
                    self.advance();
                }
                Token::RParen => break,
                _ => {
                    return Err(ParseError::Syntax {
                        offset: self.peek_offset(),
                        message: "expected ',' or ')'".into(),
                    })
                }
            }
        }
        self.expect(&Token::RParen)?;
        Ok(Statement::CreateTable { name, columns })
    }

    fn parse_drop(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Drop)?;
        let offset = self.peek_offset();
        match self.advance() {
            Token::Table => Ok(Statement::DropTable {
                name: self.expect_identifier()?,
            }),
            Token::Index => Ok(Statement::DropIndex {
                name: self.expect_identifier()?,
            }),
            Token::Policy => {
                let name = self.expect_identifier()?;
                self.expect(&Token::On)?;
                let table = self.expect_identifier()?;
                Ok(Statement::DropPolicy { name, table })
            }
            other => Err(ParseError::Syntax {
                offset,
                message: format!("expected TABLE, INDEX, or POLICY after DROP, found {other:?}"),
            }),
        }
    }

    fn parse_primary_expr(&mut self) -> Result<Expr, ParseError> {
        let offset = self.peek_offset();
        let mut expr = match self.advance() {
            Token::IntLiteral(n) => Expr::IntLiteral(n),
            Token::FloatLiteral(f) => Expr::FloatLiteral(f),
            Token::StringLiteral(s) => Expr::StringLiteral(s),
            Token::True => Expr::BoolLiteral(true),
            Token::False => Expr::BoolLiteral(false),
            Token::Null => Expr::Null,
            Token::CosineDistance | Token::L2Distance | Token::InnerProduct => {
                let metric = match self.tokens[self.pos - 1].token {
                    Token::CosineDistance => crate::sql::ast::VectorMetric::Cosine,
                    Token::L2Distance => crate::sql::ast::VectorMetric::L2,
                    Token::InnerProduct => crate::sql::ast::VectorMetric::InnerProduct,
                    _ => unreachable!(),
                };
                self.expect(&Token::LParen)?;
                let left = self.parse_where_expr()?;
                self.expect(&Token::Comma)?;
                let right = self.parse_where_expr()?;
                self.expect(&Token::RParen)?;
                Expr::VectorDistance {
                    metric,
                    left: Box::new(left),
                    right: Box::new(right),
                }
            }
            Token::FtsMatch | Token::FtsRank | Token::FtsSnippet => {
                let token_kind = self.tokens[self.pos - 1].token.clone();
                self.expect(&Token::LParen)?;
                let expr = self.parse_where_expr()?;
                self.expect(&Token::Comma)?;
                let q_offset = self.peek_offset();
                let query = match self.advance() {
                    Token::StringLiteral(s) => s,
                    other => {
                        return Err(ParseError::Syntax {
                            offset: q_offset,
                            message: format!(
                                "expected string query in text search, found {other:?}"
                            ),
                        })
                    }
                };
                self.expect(&Token::RParen)?;
                match token_kind {
                    Token::FtsMatch => Expr::FtsMatch {
                        expr: Box::new(expr),
                        query,
                    },
                    Token::FtsRank => Expr::FtsRank {
                        expr: Box::new(expr),
                        query,
                    },
                    _ => Expr::FtsSnippet {
                        expr: Box::new(expr),
                        query,
                    },
                }
            }
            Token::Exists => {
                self.expect(&Token::LParen)?;
                let subquery = self.parse_select()?;
                self.expect(&Token::RParen)?;
                Expr::Exists {
                    subquery: Box::new(subquery),
                    negated: false,
                }
            }
            Token::Not => {
                if matches!(self.peek(), Token::Exists) {
                    self.advance();
                    self.expect(&Token::LParen)?;
                    let subquery = self.parse_select()?;
                    self.expect(&Token::RParen)?;
                    Expr::Exists {
                        subquery: Box::new(subquery),
                        negated: true,
                    }
                } else {
                    return Err(ParseError::Syntax {
                        offset,
                        message: "unexpected NOT in expression".into(),
                    });
                }
            }
            Token::Count => {
                self.expect(&Token::LParen)?;
                if matches!(self.peek(), Token::Star) {
                    self.advance();
                    self.expect(&Token::RParen)?;
                    Expr::Aggregate(crate::sql::ast::AggregateFunc::CountStar)
                } else {
                    let e = self.parse_where_expr()?;
                    self.expect(&Token::RParen)?;
                    Expr::Aggregate(crate::sql::ast::AggregateFunc::Count(Box::new(e)))
                }
            }
            Token::Sum => {
                self.expect(&Token::LParen)?;
                let e = self.parse_where_expr()?;
                self.expect(&Token::RParen)?;
                Expr::Aggregate(crate::sql::ast::AggregateFunc::Sum(Box::new(e)))
            }
            Token::Avg => {
                self.expect(&Token::LParen)?;
                let e = self.parse_where_expr()?;
                self.expect(&Token::RParen)?;
                Expr::Aggregate(crate::sql::ast::AggregateFunc::Avg(Box::new(e)))
            }
            Token::Min => {
                self.expect(&Token::LParen)?;
                let e = self.parse_where_expr()?;
                self.expect(&Token::RParen)?;
                Expr::Aggregate(crate::sql::ast::AggregateFunc::Min(Box::new(e)))
            }
            Token::Max => {
                self.expect(&Token::LParen)?;
                let e = self.parse_where_expr()?;
                self.expect(&Token::RParen)?;
                Expr::Aggregate(crate::sql::ast::AggregateFunc::Max(Box::new(e)))
            }
            Token::JsonExtract => {
                self.expect(&Token::LParen)?;
                let inner = self.parse_where_expr()?;
                self.expect(&Token::Comma)?;
                let path_offset = self.peek_offset();
                let path = match self.advance() {
                    Token::StringLiteral(s) => s,
                    other => {
                        return Err(ParseError::Syntax {
                            offset: path_offset,
                            message: format!(
                                "expected string literal path in JSON_EXTRACT, found {other:?}"
                            ),
                        })
                    }
                };
                self.expect(&Token::RParen)?;
                Expr::JsonExtract {
                    expr: Box::new(inner),
                    path,
                    as_text: true,
                }
            }
            Token::AuthUid => {
                if matches!(self.peek(), Token::LParen) {
                    self.advance();
                    self.expect(&Token::RParen)?;
                }
                Expr::AuthUid
            }
            Token::Identifier(name) => {
                if name.eq_ignore_ascii_case("auth") && matches!(self.peek(), Token::Dot) {
                    self.advance();
                    let fn_name = self.expect_identifier()?;
                    if fn_name.eq_ignore_ascii_case("uid") {
                        if matches!(self.peek(), Token::LParen) {
                            self.advance();
                            self.expect(&Token::RParen)?;
                        }
                        Expr::AuthUid
                    } else {
                        Expr::QualifiedColumn {
                            table: name,
                            column: fn_name,
                        }
                    }
                } else if matches!(self.peek(), Token::Dot) {
                    self.advance();
                    let col = self.expect_identifier()?;
                    Expr::QualifiedColumn {
                        table: name,
                        column: col,
                    }
                } else {
                    Expr::Column(name)
                }
            }
            Token::Role => Expr::Column("role".into()),
            Token::User => Expr::Column("user".into()),
            Token::Password => Expr::Column("password".into()),
            Token::LParen => {
                let e = self.parse_where_expr()?;
                self.expect(&Token::RParen)?;
                e
            }
            other => {
                return Err(ParseError::Syntax {
                    offset,
                    message: format!("expected an expression, found {other:?}"),
                })
            }
        };

        while matches!(self.peek(), Token::Arrow | Token::ArrowText) {
            let as_text = matches!(self.advance(), Token::ArrowText);
            let path_offset = self.peek_offset();
            let path = match self.advance() {
                Token::StringLiteral(s) => s,
                Token::Identifier(id) => format!("$.{id}"),
                other => {
                    return Err(ParseError::Syntax {
                        offset: path_offset,
                        message: format!("expected JSON path after arrow, found {other:?}"),
                    })
                }
            };
            expr = Expr::JsonExtract {
                expr: Box::new(expr),
                path,
                as_text,
            };
        }

        Ok(expr)
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
                    Token::Comma => {
                        self.advance();
                    }
                    Token::RParen => break,
                    _ => {
                        return Err(ParseError::Syntax {
                            offset: self.peek_offset(),
                            message: "expected ',' or ')'".into(),
                        })
                    }
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
                    Token::Comma => {
                        self.advance();
                    }
                    Token::RParen => break,
                    _ => {
                        return Err(ParseError::Syntax {
                            offset: self.peek_offset(),
                            message: "expected ',' or ')'".into(),
                        })
                    }
                }
            }
            self.expect(&Token::RParen)?;
            rows.push(row);
            match self.peek() {
                Token::Comma => {
                    self.advance();
                }
                _ => break,
            }
        }
        let returning = self.parse_returning_clause()?;
        Ok(Statement::Insert {
            table,
            columns,
            rows,
            returning,
        })
    }

    fn parse_returning_clause(&mut self) -> Result<Option<SelectColumns>, ParseError> {
        if matches!(self.peek(), Token::Returning) {
            self.advance();
            if matches!(self.peek(), Token::Star) {
                self.advance();
                Ok(Some(SelectColumns::All))
            } else {
                let mut items = Vec::new();
                loop {
                    let expr = self.parse_where_expr()?;
                    let alias = if matches!(self.peek(), Token::As) {
                        self.advance();
                        Some(self.expect_identifier()?)
                    } else {
                        None
                    };
                    items.push(crate::sql::ast::SelectItem::Expr { expr, alias });
                    match self.peek() {
                        Token::Comma => {
                            self.advance();
                        }
                        _ => break,
                    }
                }
                Ok(Some(SelectColumns::Items(items)))
            }
        } else {
            Ok(None)
        }
    }

    fn parse_where_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and()?;
        while matches!(self.peek(), Token::Or) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::BinaryOp {
                op: BinOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_comparison()?;
        while matches!(self.peek(), Token::And) {
            self.advance();
            let right = self.parse_comparison()?;
            left = Expr::BinaryOp {
                op: BinOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
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
                Ok(Expr::BinaryOp {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                })
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
                Ok(Expr::IsNull {
                    expr: Box::new(left),
                    negated,
                })
            }
            Token::In => {
                self.advance();
                self.expect(&Token::LParen)?;
                if matches!(self.peek(), Token::Select) {
                    let subquery = self.parse_select()?;
                    self.expect(&Token::RParen)?;
                    Ok(Expr::InSubquery {
                        expr: Box::new(left),
                        subquery: Box::new(subquery),
                        negated: false,
                    })
                } else {
                    let mut list = Vec::new();
                    loop {
                        list.push(self.parse_where_expr()?);
                        match self.peek() {
                            Token::Comma => {
                                self.advance();
                            }
                            Token::RParen => break,
                            _ => {
                                return Err(ParseError::Syntax {
                                    offset: self.peek_offset(),
                                    message: "expected ',' or ')' in IN list".into(),
                                })
                            }
                        }
                    }
                    self.expect(&Token::RParen)?;
                    Ok(Expr::InList {
                        expr: Box::new(left),
                        list,
                        negated: false,
                    })
                }
            }
            Token::Like => {
                self.advance();
                let path_offset = self.peek_offset();
                let pattern = match self.advance() {
                    Token::StringLiteral(s) => s,
                    other => {
                        return Err(ParseError::Syntax {
                            offset: path_offset,
                            message: format!("expected pattern string after LIKE, found {other:?}"),
                        })
                    }
                };
                Ok(Expr::Like {
                    expr: Box::new(left),
                    pattern,
                    negated: false,
                })
            }
            Token::Not => {
                self.advance();
                match self.peek() {
                    Token::In => {
                        self.advance();
                        self.expect(&Token::LParen)?;
                        if matches!(self.peek(), Token::Select) {
                            let subquery = self.parse_select()?;
                            self.expect(&Token::RParen)?;
                            Ok(Expr::InSubquery {
                                expr: Box::new(left),
                                subquery: Box::new(subquery),
                                negated: true,
                            })
                        } else {
                            let mut list = Vec::new();
                            loop {
                                list.push(self.parse_where_expr()?);
                                match self.peek() {
                                    Token::Comma => {
                                        self.advance();
                                    }
                                    Token::RParen => break,
                                    _ => {
                                        return Err(ParseError::Syntax {
                                            offset: self.peek_offset(),
                                            message: "expected ',' or ')' in IN list".into(),
                                        })
                                    }
                                }
                            }
                            self.expect(&Token::RParen)?;
                            Ok(Expr::InList {
                                expr: Box::new(left),
                                list,
                                negated: true,
                            })
                        }
                    }
                    Token::Like => {
                        self.advance();
                        let path_offset = self.peek_offset();
                        let pattern = match self.advance() {
                            Token::StringLiteral(s) => s,
                            other => {
                                return Err(ParseError::Syntax {
                                    offset: path_offset,
                                    message: format!(
                                        "expected pattern string after LIKE, found {other:?}"
                                    ),
                                })
                            }
                        };
                        Ok(Expr::Like {
                            expr: Box::new(left),
                            pattern,
                            negated: true,
                        })
                    }
                    other => Err(ParseError::Syntax {
                        offset: self.peek_offset(),
                        message: format!("expected IN or LIKE after NOT, found {other:?}"),
                    }),
                }
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
            let mut items = Vec::new();
            loop {
                let expr = self.parse_where_expr()?;
                let alias = if matches!(self.peek(), Token::As) {
                    self.advance();
                    Some(self.expect_identifier()?)
                } else {
                    None
                };
                items.push(crate::sql::ast::SelectItem::Expr { expr, alias });
                match self.peek() {
                    Token::Comma => {
                        self.advance();
                    }
                    _ => break,
                }
            }
            SelectColumns::Items(items)
        };
        self.expect(&Token::From)?;
        let table = self.expect_identifier()?;

        let mut table_ref = crate::sql::ast::TableRef::Table {
            name: table.clone(),
            alias: None,
        };

        // Parse optional JOIN clauses
        while matches!(
            self.peek(),
            Token::Join | Token::Inner | Token::Left | Token::Right | Token::Cross
        ) {
            let join_type = match self.advance() {
                Token::Inner => {
                    self.expect(&Token::Join)?;
                    crate::sql::ast::JoinType::Inner
                }
                Token::Left => {
                    if matches!(self.peek(), Token::Join) {
                        self.advance();
                    }
                    crate::sql::ast::JoinType::Left
                }
                Token::Right => {
                    if matches!(self.peek(), Token::Join) {
                        self.advance();
                    }
                    crate::sql::ast::JoinType::Right
                }
                Token::Cross => {
                    self.expect(&Token::Join)?;
                    crate::sql::ast::JoinType::Cross
                }
                Token::Join => crate::sql::ast::JoinType::Inner,
                _ => unreachable!(),
            };
            let right_table = self.expect_identifier()?;
            let right_alias = if matches!(self.peek(), Token::As) {
                self.advance();
                Some(self.expect_identifier()?)
            } else {
                None
            };
            let right_ref = crate::sql::ast::TableRef::Table {
                name: right_table,
                alias: right_alias,
            };
            let condition = if join_type != crate::sql::ast::JoinType::Cross {
                self.expect(&Token::On)?;
                Some(self.parse_where_expr()?)
            } else {
                None
            };
            table_ref = crate::sql::ast::TableRef::Join {
                left: Box::new(table_ref),
                right: Box::new(right_ref),
                join_type,
                condition,
            };
        }

        let where_clause = if matches!(self.peek(), Token::Where) {
            self.advance();
            Some(self.parse_where_expr()?)
        } else {
            None
        };

        let group_by = if matches!(self.peek(), Token::Group) {
            self.advance();
            self.expect(&Token::By)?;
            let mut groups = Vec::new();
            loop {
                groups.push(self.parse_where_expr()?);
                match self.peek() {
                    Token::Comma => {
                        self.advance();
                    }
                    _ => break,
                }
            }
            Some(groups)
        } else {
            None
        };

        let having = if matches!(self.peek(), Token::Having) {
            self.advance();
            Some(self.parse_where_expr()?)
        } else {
            None
        };

        let order_by = if matches!(self.peek(), Token::Order) {
            self.advance();
            self.expect(&Token::By)?;
            let mut col = self.expect_identifier()?;
            if matches!(self.peek(), Token::Dot) {
                self.advance();
                let sub = self.expect_identifier()?;
                col = format!("{col}.{sub}");
            }
            let desc = match self.peek() {
                Token::Desc => {
                    self.advance();
                    true
                }
                Token::Asc => {
                    self.advance();
                    false
                }
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
                other => {
                    return Err(ParseError::Syntax {
                        offset,
                        message: format!("expected integer after LIMIT, found {other:?}"),
                    })
                }
            }
        } else {
            None
        };

        Ok(Statement::Select {
            columns,
            table,
            table_ref: Some(table_ref),
            where_clause,
            group_by,
            having,
            order_by,
            limit,
        })
    }

    fn parse_create_index(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Index)?;
        let name = self.expect_identifier()?;
        self.expect(&Token::On)?;
        let table = self.expect_identifier()?;
        self.expect(&Token::LParen)?;
        let column = self.expect_identifier()?;
        self.expect(&Token::RParen)?;
        Ok(Statement::CreateIndex {
            name,
            table,
            column,
        })
    }

    fn parse_update(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Update)?;
        let table = self.expect_identifier()?;
        self.expect(&Token::Set)?;
        let mut assignments = Vec::new();
        loop {
            let col = self.expect_identifier()?;
            self.expect(&Token::Eq)?;
            let value = self.parse_primary_expr()?;
            assignments.push((col, value));
            match self.peek() {
                Token::Comma => {
                    self.advance();
                }
                _ => break,
            }
        }
        let where_clause = if matches!(self.peek(), Token::Where) {
            self.advance();
            Some(self.parse_where_expr()?)
        } else {
            None
        };
        let returning = self.parse_returning_clause()?;
        Ok(Statement::Update {
            table,
            assignments,
            where_clause,
            returning,
        })
    }

    fn parse_delete(&mut self) -> Result<Statement, ParseError> {
        self.expect(&Token::Delete)?;
        self.expect(&Token::From)?;
        let table = self.expect_identifier()?;
        let where_clause = if matches!(self.peek(), Token::Where) {
            self.advance();
            Some(self.parse_where_expr()?)
        } else {
            None
        };
        let returning = self.parse_returning_clause()?;
        Ok(Statement::Delete {
            table,
            where_clause,
            returning,
        })
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
        let stmt =
            parse("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL)").unwrap();
        assert_eq!(
            stmt,
            Statement::CreateTable {
                name: "users".into(),
                columns: vec![
                    ColumnDef {
                        name: "id".into(),
                        ty: ColumnType::Integer,
                        not_null: false,
                        primary_key: true
                    },
                    ColumnDef {
                        name: "name".into(),
                        ty: ColumnType::Text,
                        not_null: true,
                        primary_key: false
                    },
                ],
            }
        );
    }

    #[test]
    fn parses_drop_table() {
        assert_eq!(
            parse("DROP TABLE users").unwrap(),
            Statement::DropTable {
                name: "users".into()
            }
        );
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
                returning: None,
            }
        );
    }

    #[test]
    fn parses_insert_with_returning() {
        let stmt =
            parse("INSERT INTO users (id, name) VALUES (1, 'Ada') RETURNING id, name").unwrap();
        assert_eq!(
            stmt,
            Statement::Insert {
                table: "users".into(),
                columns: Some(vec!["id".into(), "name".into()]),
                rows: vec![vec![Expr::IntLiteral(1), Expr::StringLiteral("Ada".into())]],
                returning: Some(SelectColumns::Items(vec![
                    SelectItem::Expr {
                        expr: Expr::Column("id".into()),
                        alias: None,
                    },
                    SelectItem::Expr {
                        expr: Expr::Column("name".into()),
                        alias: None,
                    }
                ])),
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
                returning: None,
            }
        );
    }

    #[test]
    fn parses_select_star_with_where_and_precedence() {
        // Confirms AND binds tighter than OR: `a OR b AND c` parses as `a OR (b AND c)`.
        let stmt = parse("SELECT * FROM t WHERE a = 1 OR b = 2 AND c = 3").unwrap();
        match stmt {
            Statement::Select {
                where_clause:
                    Some(Expr::BinaryOp {
                        op: BinOp::Or,
                        left,
                        right,
                    }),
                ..
            } => {
                assert_eq!(
                    *left,
                    Expr::BinaryOp {
                        op: BinOp::Eq,
                        left: Box::new(Expr::Column("a".into())),
                        right: Box::new(Expr::IntLiteral(1)),
                    }
                );
                assert!(matches!(*right, Expr::BinaryOp { op: BinOp::And, .. }));
            }
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn parses_select_with_is_null() {
        let stmt = parse("SELECT id FROM t WHERE name IS NOT NULL").unwrap();
        match stmt {
            Statement::Select {
                columns,
                table,
                where_clause,
                order_by,
                limit,
                ..
            } => {
                assert_eq!(
                    columns,
                    SelectColumns::Items(vec![SelectItem::Expr {
                        expr: Expr::Column("id".into()),
                        alias: None
                    }])
                );
                assert_eq!(table, "t");
                assert_eq!(
                    where_clause,
                    Some(Expr::IsNull {
                        expr: Box::new(Expr::Column("name".into())),
                        negated: true
                    })
                );
                assert_eq!(order_by, None);
                assert_eq!(limit, None);
            }
            other => panic!("unexpected statement: {other:?}"),
        }
    }

    #[test]
    fn parses_select_with_order_by_and_limit() {
        let stmt = parse("SELECT * FROM t ORDER BY id DESC LIMIT 10").unwrap();
        match stmt {
            Statement::Select {
                order_by, limit, ..
            } => {
                assert_eq!(order_by, Some(("id".into(), true)));
                assert_eq!(limit, Some(10));
            }
            other => panic!("unexpected parse result: {other:?}"),
        }
    }

    #[test]
    fn parses_update() {
        let stmt =
            parse("UPDATE t SET name = 'Bea', active = FALSE WHERE id = 1 RETURNING *").unwrap();
        assert_eq!(
            stmt,
            Statement::Update {
                table: "t".into(),
                assignments: vec![
                    ("name".into(), Expr::StringLiteral("Bea".into())),
                    ("active".into(), Expr::BoolLiteral(false)),
                ],
                where_clause: Some(Expr::BinaryOp {
                    op: BinOp::Eq,
                    left: Box::new(Expr::Column("id".into())),
                    right: Box::new(Expr::IntLiteral(1)),
                }),
                returning: Some(SelectColumns::All),
            }
        );
    }

    #[test]
    fn parses_delete() {
        let stmt = parse("DELETE FROM t WHERE id = 1 RETURNING id").unwrap();
        assert_eq!(
            stmt,
            Statement::Delete {
                table: "t".into(),
                where_clause: Some(Expr::BinaryOp {
                    op: BinOp::Eq,
                    left: Box::new(Expr::Column("id".into())),
                    right: Box::new(Expr::IntLiteral(1)),
                }),
                returning: Some(SelectColumns::Items(vec![SelectItem::Expr {
                    expr: Expr::Column("id".into()),
                    alias: None,
                }])),
            }
        );
    }

    #[test]
    fn parses_create_index_and_drop_index() {
        assert_eq!(
            parse("CREATE INDEX idx_name ON t (name)").unwrap(),
            Statement::CreateIndex {
                name: "idx_name".into(),
                table: "t".into(),
                column: "name".into()
            }
        );
        assert_eq!(
            parse("DROP INDEX idx_name").unwrap(),
            Statement::DropIndex {
                name: "idx_name".into()
            }
        );
    }
}
