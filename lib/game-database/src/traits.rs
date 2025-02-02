use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    statement::{QueryProperties, StatementDriverProps},
    types::DbType,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DbKind {
    MySql,
}

#[async_trait::async_trait]
pub trait DbInterface: Sync + Send {
    fn kinds(&self) -> HashSet<DbKind>;

    /// This is used to setup the database, all statements
    /// should only be related to setting up the database.
    /// If altering existing tables or similar stuff is required
    /// it should use a higher version index.
    async fn setup(
        &self,
        version_name: &str,
        versioned_stmts: BTreeMap<i64, Vec<u64>>,
    ) -> anyhow::Result<()>;

    /// Prepare a new statement.
    /// Returns an unique identifier for that statement
    #[must_use]
    async fn prepare_statement(
        &self,
        query_props: &QueryProperties,
        driver_props: &HashMap<DbKind, StatementDriverProps>,
    ) -> anyhow::Result<u64>;

    /// Drops a statement by name.
    /// If the statement does not exist, nothing happens.
    fn drop_statement(&self, unique_id: u64);

    async fn fetch_optional(
        &self,
        unique_id: u64,
        args: Vec<DbType>,
    ) -> anyhow::Result<Option<HashMap<String, DbType>>>;

    async fn fetch_one(
        &self,
        unique_id: u64,
        args: Vec<DbType>,
    ) -> anyhow::Result<HashMap<String, DbType>>;

    async fn fetch_all(
        &self,
        unique_id: u64,
        args: Vec<DbType>,
    ) -> anyhow::Result<Vec<HashMap<String, DbType>>>;

    /// Returns the affected rows count
    async fn execute(&self, unique_id: u64, args: Vec<DbType>) -> anyhow::Result<u64>;
}

pub trait DbStatementArgIndexInterface<AI> {
    fn arg_indices() -> AI;
}

pub trait DbStatementArgInterface {
    fn to_db_args(&self) -> Vec<DbType>;
}

pub trait DbStatementResultInterface {
    fn new(results: HashMap<String, DbType>) -> anyhow::Result<Self>
    where
        Self: Sized;

    fn mapping() -> HashMap<String, DbType>;
}

impl DbStatementArgIndexInterface<()> for () {
    fn arg_indices() {}
}

impl DbStatementArgInterface for () {
    fn to_db_args(&self) -> Vec<DbType> {
        vec![]
    }
}

impl DbStatementResultInterface for () {
    fn new(results: HashMap<String, DbType>) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        anyhow::ensure!(
            results.is_empty(),
            "for a empty result, the result set should be empty as well."
        );
        Ok(())
    }

    fn mapping() -> HashMap<String, DbType> {
        Default::default()
    }
}
