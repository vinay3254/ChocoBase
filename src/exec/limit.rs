use crate::error::ExecError;
use crate::exec::Operator;
use crate::storage::pager::Pager;
use crate::types::value::Value;

pub struct Limit {
    input: Box<dyn Operator>,
    remaining: i64,
}

impl Limit {
    pub fn new(input: Box<dyn Operator>, n: i64) -> Self {
        Limit { input, remaining: n }
    }
}

impl Operator for Limit {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        if self.remaining <= 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        self.input.next(pager)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    struct Fixed(std::vec::IntoIter<Vec<Value>>);
    impl Operator for Fixed {
        fn next(&mut self, _pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
            Ok(self.0.next())
        }
    }

    #[test]
    fn stops_after_n_rows() {
        let input = Fixed(vec![vec![Value::Integer(1)], vec![Value::Integer(2)], vec![Value::Integer(3)]].into_iter());
        let mut limit = Limit::new(Box::new(input), 2);
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        let mut seen = Vec::new();
        while let Some(row) = limit.next(&mut pager).unwrap() {
            seen.push(row[0].clone());
        }
        assert_eq!(seen, vec![Value::Integer(1), Value::Integer(2)]);
    }

    #[test]
    fn zero_limit_yields_nothing() {
        let input = Fixed(vec![vec![Value::Integer(1)]].into_iter());
        let mut limit = Limit::new(Box::new(input), 0);
        let file = NamedTempFile::new().unwrap();
        let mut pager = Pager::create(file.path()).unwrap();
        assert_eq!(limit.next(&mut pager).unwrap(), None);
    }
}
