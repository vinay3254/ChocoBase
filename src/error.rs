use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a valid database file")]
    NotADatabase,
    #[error("corrupt page {0}: {1}")]
    CorruptPage(u32, String),
    #[error("database file is corrupted: {0}")]
    CorruptDatabase(String),
    #[error("journal file is corrupted: {0}")]
    CorruptJournal(String),
    #[error("page number {0} is out of bounds")]
    PageOutOfBounds(u32),
    #[error("buffer pool is full with pinned frames")]
    BufferPoolFull,
    #[error("database is locked by another active process ({0})")]
    DatabaseLocked(String),
}

#[derive(Debug, Error)]
pub enum BTreeError {
    #[error("duplicate key")]
    DuplicateKey,
    #[error("row size {0} exceeds max allowable page payload {1}")]
    RowTooLarge(usize, usize),
    #[error("key size {0} exceeds maximum allowable length")]
    KeyTooLarge(usize),
    #[error("value size {0} exceeds maximum allowable length")]
    ValueTooLarge(usize),
    #[error("node {0} has corrupt or invalid format")]
    CorruptNode(u32),
    #[error("cannot split root node: no free pages available")]
    NoFreePages,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("parse error at byte {offset}: {message}")]
    Syntax { offset: usize, message: String },
}

#[derive(Debug, Error)]
pub enum PlanError {
    #[error("no such table: {0}")]
    NoSuchTable(String),
    #[error("no such column: {0}")]
    NoSuchColumn(String),
    #[error("no such index: {0}")]
    NoSuchIndex(String),
    #[error("table already exists: {0}")]
    TableAlreadyExists(String),
    #[error("index already exists: {0}")]
    IndexAlreadyExists(String),
    #[error("invalid schema: {0}")]
    InvalidSchema(String),
    #[error("invalid expression: {0}")]
    InvalidExpression(String),
    #[error("column count mismatch: expected {expected}, found {found}")]
    ColumnCountMismatch { expected: usize, found: usize },
    #[error("nested transactions are not supported")]
    NestedTransactionNotSupported,
    #[error("cannot commit or rollback: no transaction is in progress")]
    NoTransactionInProgress,
}

#[derive(Debug, Error)]
pub enum ExecError {
    #[error("NOT NULL constraint failed: {0}")]
    NotNullViolation(String),
    #[error("duplicate primary key")]
    DuplicatePrimaryKey,
    #[error("invalid value: {0}")]
    InvalidValue(String),
    #[error(transparent)]
    BTree(#[from] BTreeError),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

#[derive(Debug, Error)]
pub enum DbError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    BTree(#[from] BTreeError),
    #[error(transparent)]
    Parse(#[from] ParseError),
    #[error(transparent)]
    Plan(#[from] PlanError),
    #[error(transparent)]
    Exec(#[from] ExecError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, DbError>;
