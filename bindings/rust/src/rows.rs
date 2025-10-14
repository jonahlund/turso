use turso_core::types::FromValue;

use crate::{Error, Result, Value};
use std::fmt::Debug;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::task::Poll;

/// Results of a prepared statement query.
pub struct Rows {
    inner: Arc<Mutex<turso_core::Statement>>,
}

impl Clone for Rows {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

unsafe impl Send for Rows {}
unsafe impl Sync for Rows {}

impl Rows {
    pub(crate) fn new(inner: &Arc<Mutex<turso_core::Statement>>) -> Self {
        Self {
            inner: Arc::clone(inner),
        }
    }
    /// Fetch the next row of this result set.
    pub async fn next(&mut self) -> Result<Option<Row>> {
        struct Next {
            stmt: Arc<Mutex<turso_core::Statement>>,
        }

        impl Future for Next {
            type Output = Result<Option<Row>>;

            fn poll(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Self::Output> {
                let mut stmt = self
                    .stmt
                    .lock()
                    .map_err(|e| Error::MutexError(e.to_string()))?;
                match stmt.step_with_waker(cx.waker())? {
                    turso_core::StepResult::Row => {
                        let row = stmt.row().unwrap();
                        Poll::Ready(Ok(Some(Row {
                            values: row.get_values().map(|v| v.to_owned()).collect(),
                            names: (0..stmt.num_columns())
                                .map(|idx| stmt.get_column_name(idx).into_owned())
                                .collect(),
                        })))
                    }
                    turso_core::StepResult::Done => Poll::Ready(Ok(None)),
                    turso_core::StepResult::IO => {
                        stmt.run_once()?;
                        Poll::Pending
                    }
                    turso_core::StepResult::Busy => Poll::Ready(Err(Error::SqlExecutionFailure(
                        "database is locked".to_string(),
                    ))),
                    turso_core::StepResult::Interrupt => {
                        Poll::Ready(Err(Error::SqlExecutionFailure("interrupted".to_string())))
                    }
                }
            }
        }

        unsafe impl Send for Next {}

        let next = Next {
            stmt: self.inner.clone(),
        };

        next.await
    }
}

/// Query result row.
#[derive(Debug)]
pub struct Row {
    values: Vec<turso_core::Value>,
    names: Vec<String>,
}

unsafe impl Send for Row {}
unsafe impl Sync for Row {}

impl Row {
    pub fn get_value<I>(&self, idx: I) -> Result<Value>
    where
        I: RowIndex,
    {
        let idx = idx.idx(self)?;
        let value = &self.values[idx];
        match value {
            turso_core::Value::Integer(i) => Ok(Value::Integer(*i)),
            turso_core::Value::Null => Ok(Value::Null),
            turso_core::Value::Float(f) => Ok(Value::Real(*f)),
            turso_core::Value::Text(text) => Ok(Value::Text(text.to_string())),
            turso_core::Value::Blob(items) => Ok(Value::Blob(items.to_vec())),
        }
    }

    pub fn get<I, T>(&self, idx: I) -> Result<T>
    where
        I: RowIndex,
        T: FromValue,
    {
        let idx = idx.idx(self)?;
        let val = &self.values[idx];
        T::from_sql(val.clone()).map_err(|err| Error::ConversionFailure(err.to_string()))
    }

    pub fn column_count(&self) -> usize {
        self.values.len()
    }

    pub fn column_index(&self, name: &str) -> Result<usize> {
        let idx = self
            .names
            .iter()
            .position(|n| n == name)
            .ok_or_else(|| Error::InvalidColumnName(name.to_string()))?;

        if idx > self.column_count() {
            return Err(Error::InvalidColumnIndex(idx));
        }

        Ok(idx)
    }
}

impl<'a> FromIterator<&'a turso_core::Value> for Row {
    fn from_iter<T: IntoIterator<Item = &'a turso_core::Value>>(iter: T) -> Self {
        let values = iter
            .into_iter()
            .map(|v| match v {
                turso_core::Value::Integer(i) => turso_core::Value::Integer(*i),
                turso_core::Value::Null => turso_core::Value::Null,
                turso_core::Value::Float(f) => turso_core::Value::Float(*f),
                turso_core::Value::Text(s) => turso_core::Value::Text(s.clone()),
                turso_core::Value::Blob(b) => turso_core::Value::Blob(b.clone()),
            })
            .collect();

        Row {
            values,
            names: vec![],
        }
    }
}

pub trait RowIndex {
    fn idx(&self, row: &Row) -> Result<usize>;
}

impl RowIndex for usize {
    #[inline]
    fn idx(&self, row: &Row) -> Result<usize> {
        if *self >= row.column_count() {
            Err(Error::InvalidColumnIndex(*self))
        } else {
            Ok(*self)
        }
    }
}

impl RowIndex for &'_ str {
    #[inline]
    fn idx(&self, row: &Row) -> Result<usize> {
        row.column_index(self)
    }
}
