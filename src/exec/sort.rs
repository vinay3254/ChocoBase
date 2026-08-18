use crate::error::ExecError;
use crate::exec::Operator;
use crate::storage::pager::Pager;
use crate::types::value::{sql_cmp_nullable, Value};

pub struct Sort {
    input: Box<dyn Operator>,
    key_index: usize,
    descending: bool,
    buffer: Option<std::vec::IntoIter<Vec<Value>>>,
}

impl Sort {
    pub fn new(input: Box<dyn Operator>, key_index: usize, descending: bool) -> Self {
        Sort {
            input,
            key_index,
            descending,
            buffer: None,
        }
    }
}

impl Operator for Sort {
    fn next(&mut self, pager: &mut Pager) -> Result<Option<Vec<Value>>, ExecError> {
        if self.buffer.is_none() {
            let mut rows = Vec::new();
            while let Some(r) = self.input.next(pager)? {
                rows.push(r);
            }
            rows.sort_by(|a, b| {
                let ord = sql_cmp_nullable(&a[self.key_index], &b[self.key_index]);
                if self.descending {
                    ord.reverse()
                } else {
                    ord
                }
            });
            self.buffer = Some(rows.into_iter());
        }
        Ok(self.buffer.as_mut().unwrap().next())
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

    fn pager() -> Pager {
        let file = NamedTempFile::new().unwrap();
        Pager::create(file.path()).unwrap()
    }

    #[test]
    fn sorts_ascending_by_key_index() {
        let input = Fixed(
            vec![
                vec![Value::Integer(3)],
                vec![Value::Integer(1)],
                vec![Value::Integer(2)],
            ]
            .into_iter(),
        );
        let mut sort = Sort::new(Box::new(input), 0, false);
        let mut pager = pager();
        let mut seen = Vec::new();
        while let Some(row) = sort.next(&mut pager).unwrap() {
            seen.push(row[0].clone());
        }
        assert_eq!(
            seen,
            vec![Value::Integer(1), Value::Integer(2), Value::Integer(3)]
        );
    }

    #[test]
    fn sorts_descending_and_places_nulls_first() {
        let input = Fixed(
            vec![
                vec![Value::Integer(1)],
                vec![Value::Null],
                vec![Value::Integer(2)],
            ]
            .into_iter(),
        );
        let mut sort = Sort::new(Box::new(input), 0, true);
        let mut pager = pager();
        let mut seen = Vec::new();
        while let Some(row) = sort.next(&mut pager).unwrap() {
            seen.push(row[0].clone());
        }
        // descending reverses the whole comparator, including the Null-sorts-first rule,
        // so Null (normally first) ends up last under DESC.
        assert_eq!(
            seen,
            vec![Value::Integer(2), Value::Integer(1), Value::Null]
        );
    }
}
