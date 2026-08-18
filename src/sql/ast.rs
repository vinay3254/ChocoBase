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
        where_clause: Option<Expr>,
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
    IntLiteral(i64),
    StringLiteral(String),
    BoolLiteral(bool),
    Null,
    BinaryOp { op: BinOp, left: Box<Expr>, right: Box<Expr> },
    IsNull { expr: Box<Expr>, negated: bool },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    And, Or, Eq, NotEq, Lt, LtEq, Gt, GtEq,
}
