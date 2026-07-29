#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Create, Table, Drop, Index, On, Insert, Into, Values, Select, From, Where,
    Update, Set, Delete, Order, By, Asc, Desc, Limit, Not, Null, Primary, Key,
    And, Or, Is,
    KwInteger, KwText, KwBoolean,
    Identifier(String),
    IntLiteral(i64),
    StringLiteral(String),
    True, False,
    Eq, NotEq, Lt, LtEq, Gt, GtEq,
    LParen, RParen, Comma, Star, Semicolon,
    Eof,
}

#[derive(Debug, Clone)]
pub struct SpannedToken {
    pub token: Token,
    pub offset: usize,
}
