use thiserror::Error;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("not a valid database file")]
    NotADatabase,
    #[error("corrupt page {0}: {1}")]
    CorruptPage(u32, String),
    #[error("page {0} out of range")]
    PageOutOfRange(u32),
    #[error("corrupt journal: {0}")]
    CorruptJournal(String),
    #[error("database is locked: {0}")]
    DatabaseLocked(String),
}

#[derive(Debug, Error)]
pub enum BTreeError {
    #[error("row too large: {0} bytes exceeds page size {1}")]
    RowTooLarge(usize, usize),
    #[error("duplicate key")]
    DuplicateKey,
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
}

pub type Result<T> = std::result::Result<T, DbError>;
