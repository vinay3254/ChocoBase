pub mod scan;
pub mod filter;
pub mod project;
pub mod sort;
pub mod limit;
pub mod mutate;
pub mod join;
pub mod aggregate;

use crate::error::ExecError;
use crate::storage::pager::Pager;
use crate::types::value::Value;

pub trait Operator {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError>;
}
