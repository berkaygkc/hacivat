//! The connection the introspector reads through: any [`RawConn`] (the driver API, HV-DRV-01),
//! over the simple query protocol, whose rows every driver returns as text
//! (`ExecOpts::values`). The spike renders only numbers into its SQL, so it needs no binds;
//! M4's packs move to the typed path with real binds.

use futures::future::BoxFuture;
use hacivat_driver::{ExecOpts, RawConn, RawError};

/// Rows as text, `None` for SQL NULL.
pub type Rows = Vec<Vec<Option<String>>>;

/// What the introspector needs from a connection.
pub trait CatalogConn: Send {
    /// Runs one query and returns every row as text.
    fn rows<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<Rows, RawError>>;
    /// Runs trusted control text (`BEGIN …`, `COMMIT`, `ROLLBACK`).
    fn control<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), RawError>>;
}

impl<T: RawConn + ?Sized> CatalogConn for T {
    fn rows<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<Rows, RawError>> {
        Box::pin(async move {
            let s = self.execute(sql, ExecOpts::values(usize::MAX)).await?;
            Ok(s.rows)
        })
    }

    fn control<'a>(&'a mut self, sql: &'a str) -> BoxFuture<'a, Result<(), RawError>> {
        self.batch(sql)
    }
}
