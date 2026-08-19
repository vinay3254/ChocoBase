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
    #[error("lock acquisition timed out / deadlock avoided")]
    LockTimeout,
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
    #[error("cannot update primary key column")]
    CannotUpdatePrimaryKey,
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
    #[error(transparent)]
    Plan(#[from] PlanError),
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

impl DbError {
    /// Returns the standard 5-character PostgreSQL SQLSTATE error code for this error.
    pub fn sqlstate(&self) -> &'static str {
        match self {
            DbError::Parse(ParseError::Syntax { .. }) => "42601", // syntax_error
            DbError::Plan(PlanError::NoSuchTable(_)) => "42P01",  // undefined_table
            DbError::Plan(PlanError::NoSuchColumn(_)) => "42703", // undefined_column
            DbError::Plan(PlanError::TableAlreadyExists(_)) => "42P07", // duplicate_table
            DbError::Plan(PlanError::IndexAlreadyExists(_)) => "42P07", // duplicate_table
            DbError::Plan(PlanError::CannotUpdatePrimaryKey) => "0A000", // feature_not_supported
            DbError::Exec(ExecError::NotNullViolation(_)) => "23502", // not_null_violation
            DbError::Exec(ExecError::DuplicatePrimaryKey) => "23505", // unique_violation
            DbError::BTree(BTreeError::DuplicateKey) => "23505",  // unique_violation
            DbError::Exec(ExecError::BTree(BTreeError::DuplicateKey)) => "23505", // unique_violation
            DbError::Exec(ExecError::InvalidValue(_)) => "22000",                 // data_exception
            DbError::Storage(StorageError::DatabaseLocked(_)) => "55P03", // lock_not_available
            DbError::Exec(ExecError::Storage(StorageError::DatabaseLocked(_))) => "55P03",
            _ => "XX000", // internal_error
        }
    }
}

pub type Result<T> = std::result::Result<T, DbError>;
