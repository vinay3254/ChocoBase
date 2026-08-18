use crate::types::value::ColumnType;

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    CreateTable { name: String, columns: Vec<ColumnDef> },
    DropTable { name: String },
    CreateIndex { name: String, table: String, column: String },
    DropIndex { name: String },
    Insert { table: String, columns: Option<Vec<String>>, rows: Vec<Vec<Expr>> },
    Select {
        columns: SelectColumns,
        table: String,
        table_ref: Option<TableRef>,
        where_clause: Option<Expr>,
        group_by: Option<Vec<Expr>>,
        having: Option<Expr>,
        order_by: Option<(String, bool)>,
        limit: Option<i64>,
    },
    Update { table: String, assignments: Vec<(String, Expr)>, where_clause: Option<Expr> },
    Delete { table: String, where_clause: Option<Expr> },
    Begin,
    Commit,
    Rollback,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectColumns {
    All,
    List(Vec<String>),
    Items(Vec<SelectItem>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SelectItem {
    All,
    Expr { expr: Expr, alias: Option<String> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum TableRef {
    Table { name: String, alias: Option<String> },
    Join {
        left: Box<TableRef>,
        right: Box<TableRef>,
        join_type: JoinType,
        condition: Option<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JoinType {
    Inner,
    Left,
    Right,
    Cross,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    pub name: String,
    pub ty: ColumnType,
    pub not_null: bool,
    pub primary_key: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Column(String),
    QualifiedColumn { table: String, column: String },
    IntLiteral(i64),
    StringLiteral(String),
    BoolLiteral(bool),
    Null,
    BinaryOp { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    IsNull { expr: Box<Expr>, negated: bool },
    Aggregate(AggregateFunc),
    JsonExtract { expr: Box<Expr>, path: String, as_text: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AggregateFunc {
    CountStar,
    Count(Box<Expr>),
    Sum(Box<Expr>),
    Avg(Box<Expr>),
    Min(Box<Expr>),
    Max(Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    And, Or, Eq, NotEq, Lt, LtEq, Gt, GtEq,
}
