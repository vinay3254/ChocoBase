#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    Create, Table, Drop, Index, On, Insert, Into, Values, Select, From, Where,
    Update, Set, Delete, Order, By, Asc, Desc, Limit, Not, Null, Primary, Key,
    And, Or, Is, Begin, Commit, Rollback, Transaction,
    Join, Inner, Left, Right, Cross, Group, Having, As,
    Count, Sum, Avg, Min, Max, JsonExtract,
    KwInteger, KwText, KwBoolean, KwJson,
    Identifier(String),
    IntLiteral(i64),
    StringLiteral(String),
    True, False,
    Eq, NotEq, Lt, LtEq, Gt, GtEq,
    Arrow, ArrowText,
    LParen, RParen, Comma, Star, Dot, Semicolon,
    Eof,
}

#[derive(Debug, Clone)]
pub struct SpannedToken {
    pub token: Token,
    pub offset: usize,
}
