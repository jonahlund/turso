use std::ops::Deref;

use crate::{Connection, Result};

/// Options for transaction behavior. See [BEGIN
/// TRANSACTION](http://www.sqlite.org/lang_transaction.html) for details.
#[derive(Copy, Clone)]
#[non_exhaustive]
pub enum TransactionBehavior {
    /// DEFERRED means that the transaction does not actually start until the
    /// database is first accessed.
    Deferred,
    /// IMMEDIATE cause the database connection to start a new write
    /// immediately, without waiting for a writes statement.
    Immediate,
    /// EXCLUSIVE prevents other database connections from reading the database
    /// while the transaction is underway.
    Exclusive,
}

/// Options for how a Transaction should behave when it is dropped.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropBehavior {
    /// Roll back the changes. This is the default.
    Rollback,

    /// Commit the changes.
    Commit,

    /// Do not commit or roll back changes - this will leave the transaction or
    /// savepoint open, so should be used with care.
    Ignore,

    /// Panic. Used to enforce intentional behavior during development.
    Panic,
}

/// Represents a transaction on a database connection.
///
/// ## Note
///
/// Transactions will roll back by default. Use `commit` method to explicitly
/// commit the transaction, or use `set_drop_behavior` to change what happens
/// when the transaction is dropped.
///
/// ## Example
///
/// ```rust,no_run
/// # use turso::{Connection, Result};
/// # fn do_queries_part_1(_conn: &Connection) -> Result<()> { Ok(()) }
/// # fn do_queries_part_2(_conn: &Connection) -> Result<()> { Ok(()) }
/// async fn perform_queries(conn: &mut Connection) -> Result<()> {
///     let tx = conn.transaction().await?;
///
///     do_queries_part_1(&tx)?; // tx causes rollback if this fails
///     do_queries_part_2(&tx)?; // tx causes rollback if this fails
///
///     tx.commit().await
/// }
/// ```
#[derive(Debug)]
pub struct Transaction<'conn> {
    conn: &'conn Connection,
    drop_behavior: DropBehavior,
    must_finish: bool,
}

impl Transaction<'_> {
    /// Begin a new transaction. Cannot be nested;
    ///
    /// Even though we don't mutate the connection, we take a `&mut Connection`
    /// to prevent nested transactions on the same connection. For cases
    /// where this is unacceptable, [`Transaction::new_unchecked`] is available.
    #[inline]
    pub async fn new(
        conn: &mut Connection,
        behavior: TransactionBehavior,
    ) -> Result<Transaction<'_>> {
        Self::new_unchecked(conn, behavior).await
    }

    /// Begin a new transaction, failing if a transaction is open.
    ///
    /// If a transaction is already open, this will return an error. Where
    /// possible, [`Transaction::new`] should be preferred, as it provides a
    /// compile-time guarantee that transactions are not nested.
    #[inline]
    pub async fn new_unchecked(
        conn: &Connection,
        behavior: TransactionBehavior,
    ) -> Result<Transaction<'_>> {
        let query = match behavior {
            TransactionBehavior::Deferred => "BEGIN DEFERRED",
            TransactionBehavior::Immediate => "BEGIN IMMEDIATE",
            TransactionBehavior::Exclusive => "BEGIN EXCLUSIVE",
        };
        // TODO: Use execute_batch instead
        conn.execute(query, ()).await.map(move |_| Transaction {
            conn,
            drop_behavior: DropBehavior::Rollback,
            must_finish: true,
        })
    }

    /// Get the current setting for what happens to the transaction when it is
    /// dropped.
    #[inline]
    #[must_use]
    pub fn drop_behavior(&self) -> DropBehavior {
        self.drop_behavior
    }

    /// Configure the transaction to perform the specified action when it is
    /// dropped.
    #[inline]
    pub fn set_drop_behavior(&mut self, drop_behavior: DropBehavior) {
        self.drop_behavior = drop_behavior;
    }

    /// A convenience method which consumes and commits a transaction.
    #[inline]
    pub async fn commit(mut self) -> Result<()> {
        self._commit().await
    }

    #[inline]
    async fn _commit(&mut self) -> Result<()> {
        self.must_finish = false;
        self.conn.execute("COMMIT", ()).await?;
        Ok(())
    }

    /// A convenience method which consumes and rolls back a transaction.
    #[inline]
    pub async fn rollback(mut self) -> Result<()> {
        self._rollback().await
    }

    #[inline]
    async fn _rollback(&mut self) -> Result<()> {
        self.must_finish = false;
        self.conn.execute("ROLLBACK", ()).await?;
        Ok(())
    }

    /// Consumes the transaction, committing or rolling back according to the
    /// current setting (see `drop_behavior`).
    ///
    /// Functionally equivalent to the `Drop` implementation, but allows
    /// callers to see any errors that occur.
    #[inline]
    pub async fn finish(mut self) -> Result<()> {
        self._finish().await
    }

    #[inline]
    async fn _finish(&mut self) -> Result<()> {
        if self.conn.is_autocommit()? {
            return Ok(());
        }
        match self.drop_behavior() {
            DropBehavior::Commit => {
                if (self._commit().await).is_err() {
                    self._rollback().await
                } else {
                    Ok(())
                }
            }
            DropBehavior::Rollback => self._rollback().await,
            DropBehavior::Ignore => Ok(()),
            DropBehavior::Panic => panic!("Transaction dropped unexpectedly."),
        }
    }
}

impl Deref for Transaction<'_> {
    type Target = Connection;

    #[inline]
    fn deref(&self) -> &Connection {
        self.conn
    }
}

impl Drop for Transaction<'_> {
    #[inline]
    fn drop(&mut self) {
        if self.must_finish {
            panic!("Transaction dropped without finish()")
        }
    }
}

impl Connection {
    /// Begin a new transaction with the default behavior (DEFERRED).
    ///
    /// The transaction defaults to rolling back when it is dropped. If you
    /// want the transaction to commit, you must call
    /// [`commit`](Transaction::commit) or
    /// [`set_drop_behavior(DropBehavior::Commit)`](Transaction::set_drop_behavior).
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use turso::{Connection, Result};
    /// # fn do_queries_part_1(_conn: &Connection) -> Result<()> { Ok(()) }
    /// # fn do_queries_part_2(_conn: &Connection) -> Result<()> { Ok(()) }
    /// async fn perform_queries(conn: &mut Connection) -> Result<()> {
    ///     let tx = conn.transaction().await?;
    ///
    ///     do_queries_part_1(&tx)?; // tx causes rollback if this fails
    ///     do_queries_part_2(&tx)?; // tx causes rollback if this fails
    ///
    ///     tx.commit().await
    /// }
    /// ```
    ///
    /// # Failure
    ///
    /// Will return `Err` if the call fails.
    #[inline]
    pub async fn transaction(&mut self) -> Result<Transaction<'_>> {
        Transaction::new(self, self.transaction_behavior).await
    }

    /// Begin a new transaction with a specified behavior.
    ///
    /// See [`transaction`](Connection::transaction).
    ///
    /// # Failure
    ///
    /// Will return `Err` if the call fails.
    #[inline]
    pub async fn transaction_with_behavior(
        &mut self,
        behavior: TransactionBehavior,
    ) -> Result<Transaction<'_>> {
        Transaction::new(self, behavior).await
    }

    /// Begin a new transaction with the default behavior (DEFERRED).
    ///
    /// Attempt to open a nested transaction will result in a SQLite error.
    /// `Connection::transaction` prevents this at compile time by taking `&mut
    /// self`, but `Connection::unchecked_transaction()` may be used to defer
    /// the checking until runtime.
    ///
    /// See [`Connection::transaction`] and [`Transaction::new_unchecked`]
    /// (which can be used if the default transaction behavior is undesirable).
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use turso::{Connection, Result};
    /// # use std::rc::Rc;
    /// # fn do_queries_part_1(_conn: &Connection) -> Result<()> { Ok(()) }
    /// # fn do_queries_part_2(_conn: &Connection) -> Result<()> { Ok(()) }
    /// async fn perform_queries(conn: Rc<Connection>) -> Result<()> {
    ///     let tx = conn.unchecked_transaction().await?;
    ///
    ///     do_queries_part_1(&tx)?; // tx causes rollback if this fails
    ///     do_queries_part_2(&tx)?; // tx causes rollback if this fails
    ///
    ///     tx.commit().await
    /// }
    /// ```
    ///
    /// # Failure
    ///
    /// Will return `Err` if the underlying SQLite call fails. The specific
    /// error returned if transactions are nested is currently unspecified.
    pub async fn unchecked_transaction(&self) -> Result<Transaction<'_>> {
        Transaction::new_unchecked(self, self.transaction_behavior).await
    }

    /// Set the default transaction behavior for the connection.
    ///
    /// ## Note
    ///
    /// This will only apply to transactions initiated by [`transaction`](Connection::transaction)
    /// or [`unchecked_transaction`](Connection::unchecked_transaction).
    ///
    /// ## Example
    ///
    /// ```rust,no_run
    /// # use turso::{Connection, Result};
    /// # use turso::transaction::TransactionBehavior;
    /// # fn do_queries_part_1(_conn: &Connection) -> Result<()> { Ok(()) }
    /// # fn do_queries_part_2(_conn: &Connection) -> Result<()> { Ok(()) }
    /// async fn perform_queries(conn: &mut Connection) -> Result<()> {
    ///     conn.set_transaction_behavior(TransactionBehavior::Immediate);
    ///
    ///     let tx = conn.transaction().await?;
    ///
    ///     do_queries_part_1(&tx)?; // tx causes rollback if this fails
    ///     do_queries_part_2(&tx)?; // tx causes rollback if this fails
    ///
    ///     tx.commit().await
    /// }
    /// ```
    pub fn set_transaction_behavior(&mut self, behavior: TransactionBehavior) {
        self.transaction_behavior = behavior;
    }
}

#[cfg(test)]
mod test {
    use crate::{Builder, Connection, Error, Result};

    use super::DropBehavior;

    async fn checked_memory_handle() -> Result<Connection> {
        let db = Builder::new_local(":memory:").build().await?;
        let conn = db.connect()?;
        conn.execute("CREATE TABLE foo (x INTEGER)", ()).await?;
        Ok(conn)
    }

    #[tokio::test]
    #[should_panic(expected = "Transaction dropped without finish()")]
    async fn test_drop_panic() {
        let mut conn = checked_memory_handle().await.unwrap();
        {
            let tx = conn.transaction().await.unwrap();
            tx.execute("INSERT INTO foo VALUES(?)", &[1]).await.unwrap();
        }
    }

    #[tokio::test]
    async fn test_drop() -> Result<()> {
        let mut conn = checked_memory_handle().await?;
        {
            let tx = conn.transaction().await?;
            tx.execute("INSERT INTO foo VALUES(?)", &[1]).await?;
            tx.finish().await?;
            // default: rollback
        }
        {
            let mut tx = conn.transaction().await?;
            tx.execute("INSERT INTO foo VALUES(?)", &[2]).await?;
            tx.set_drop_behavior(DropBehavior::Commit);
            tx.finish().await?;
        }
        {
            let tx = conn.transaction().await?;
            let result = tx
                .prepare("SELECT SUM(x) FROM foo")
                .await?
                .query_row(())
                .await?;

            assert_eq!(2, result.get::<_, i32>(0)?);
            tx.finish().await?;
        }
        Ok(())
    }

    fn assert_nested_tx_error(e: Error) {
        if let Error::SqlExecutionFailure(e) = &e {
            assert!(e.contains("transaction"));
        } else {
            panic!("Unexpected error type: {e:?}");
        }
    }

    #[tokio::test]
    async fn test_unchecked_nesting() -> Result<()> {
        let conn = checked_memory_handle().await?;

        {
            let tx = conn.unchecked_transaction().await?;
            let e = tx.unchecked_transaction().await.unwrap_err();
            assert_nested_tx_error(e);
            tx.finish().await?;
            // default: rollback
        }
        {
            let tx = conn.unchecked_transaction().await?;
            tx.execute("INSERT INTO foo VALUES(?)", &[1]).await?;
            // Ensure this doesn't interfere with ongoing transaction
            let e = tx.unchecked_transaction().await.unwrap_err();
            assert_nested_tx_error(e);

            tx.execute("INSERT INTO foo VALUES(?)", &[1]).await?;
            tx.commit().await?;
        }

        let result = conn
            .prepare("SELECT SUM(x) FROM foo")
            .await?
            .query_row(())
            .await?;
        assert_eq!(2, result.get::<_, i32>(0)?);
        Ok(())
    }

    #[tokio::test]
    async fn test_explicit_rollback_commit() -> Result<()> {
        let mut conn = checked_memory_handle().await?;
        {
            let tx = conn.transaction().await?;
            tx.execute("INSERT INTO foo VALUES(?)", &[1]).await?;
            tx.rollback().await?;

            // This is a current Turso's limitation.
            // Since we don't have support for savepoints yet,
            // a rollback ends with a transaction so we need to immediately open a new one.
            let tx = conn.transaction().await?;
            tx.execute("INSERT INTO foo VALUES(?)", &[2]).await?;
            tx.commit().await?;
        }
        {
            let tx = conn.transaction().await?;
            tx.execute("INSERT INTO foo VALUES(?)", &[4]).await?;
            tx.commit().await?;
        }
        {
            let result = conn
                .prepare("SELECT SUM(x) FROM foo")
                .await?
                .query_row(())
                .await?;
            assert_eq!(6, result.get::<_, i32>(0)?);
        }
        Ok(())
    }
}
