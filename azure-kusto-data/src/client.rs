//! This module contains the client for the Azure Kusto Data service.

use crate::authorization_policy::AuthorizationPolicy;
use crate::connection_string::ConnectionString;
use crate::error::Result;
use crate::operations::query::{QueryRunner, QueryRunnerBuilder, V1QueryRunner, V2QueryRunner};
use azure_core::auth::TokenCredential;

use azure_core::{ClientOptions, Context, Pipeline};

use crate::request_options::RequestOptions;
use std::convert::TryFrom;
use std::fmt::Debug;
use std::sync::Arc;

/// Options for specifying how a Kusto client will behave
#[derive(Clone, Default)]
pub struct KustoClientOptions {
    options: ClientOptions,
}

impl From<ClientOptions> for KustoClientOptions {
    fn from(c: ClientOptions) -> Self {
        Self { options: c }
    }
}

impl KustoClientOptions {
    /// Create new options
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

fn new_pipeline_from_options(
    credential: Arc<dyn TokenCredential>,
    resource: &str,
    options: KustoClientOptions,
) -> Pipeline {
    let auth_policy = Arc::new(AuthorizationPolicy::new(credential, resource));
    // take care of adding the AuthorizationPolicy as **last** retry policy.
    let per_retry_policies: Vec<Arc<(dyn azure_core::Policy + 'static)>> = vec![auth_policy];

    Pipeline::new(
        option_env!("CARGO_PKG_NAME"),
        option_env!("CARGO_PKG_VERSION"),
        options.options,
        Vec::new(),
        per_retry_policies,
    )
}

/// Kusto client for Rust.
/// The client is a wrapper around the Kusto REST API.
/// To read more about it, go to [https://docs.microsoft.com/en-us/azure/kusto/api/rest/](https://docs.microsoft.com/en-us/azure/kusto/api/rest/)
///
/// The primary methods are:
/// `execute_query`:  executes a KQL query against the Kusto service.
#[derive(Clone, Debug)]
pub struct KustoClient {
    pipeline: Arc<Pipeline>,
    query_url: Arc<String>,
    management_url: Arc<String>,
}

/// Denotes what kind of query is being executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryKind {
    /// A Management query. The returned type is [`KustoResponse::V1`](crate::operations::query::KustoResponse::V1)
    Management,
    /// A KQL query. The returned type is [`KustoResponse::V2`](crate::operations::query::KustoResponse::V2)
    Query,
}

impl KustoClient {
    /// Create a new Kusto client.
    /// This method accepts a connection string, that includes the Kusto cluster and the authentication information for the cluster.
    /// # Example
    /// ```rust
    /// use azure_kusto_data::prelude::*;
    ///
    /// let client = KustoClient::new(
    ///    ConnectionString::with_default_auth("https://mycluster.region.kusto.windows.net/"),
    ///    KustoClientOptions::default());
    ///
    /// assert!(client.is_ok());
    /// ```
    pub fn new(connection_string: ConnectionString, options: KustoClientOptions) -> Result<Self> {
        let (data_source, credentials) = connection_string.into_data_source_and_credentials();
        let service_url = data_source.trim_end_matches('/');
        let query_url = format!("{service_url}/v2/rest/query");
        let management_url = format!("{service_url}/v1/rest/mgmt");
        let pipeline = new_pipeline_from_options(credentials, service_url, options);

        Ok(Self {
            pipeline: pipeline.into(),
            query_url: query_url.into(),
            management_url: management_url.into(),
        })
    }

    pub(crate) fn query_url(&self) -> &str {
        &self.query_url
    }

    pub(crate) fn management_url(&self) -> &str {
        &self.management_url
    }

    pub(crate) fn pipeline(&self) -> &Pipeline {
        &self.pipeline
    }

    /// Execute a query against the Kusto cluster.
    /// The `kind` parameter determines whether the request is a query (retrieves data from the tables) or a management query (commands to monitor and manage the cluster).
    /// This method should only be used if the query kind is not known at compile time, otherwise use [execute](#method.execute) or [execute_command](#method.execute_command).
    /// # Example
    /// ```no_run
    /// use azure_kusto_data::prelude::*;
    /// # #[tokio::main] async fn main() -> Result<(), Error> {
    ///
    /// let client = KustoClient::new(
    ///   ConnectionString::with_default_auth("https://mycluster.region.kusto.windows.net/"),
    ///   KustoClientOptions::default())?;
    ///
    ///  let result = client.execute_with_options("some_database", ".show version", QueryKind::Management, None).await?;
    ///
    /// assert!(matches!(result, KustoResponse::V1(..)));
    /// # Ok(())}
    /// ```
    #[must_use]
    pub fn execute_with_options(
        &self,
        database: impl Into<String>,
        query: impl Into<String>,
        kind: QueryKind,
        options: impl Into<Option<RequestOptions>>,
    ) -> QueryRunner {
        QueryRunnerBuilder::default()
            .with_kind(kind)
            .with_client(self.clone())
            .with_database(database)
            .with_query(query)
            .with_context(Context::new())
            .with_options(options)
            .build()
            .expect("Unexpected error when building query runner - please report this issue to the Kusto team")
    }

    /// Execute a KQL query with additional request options.
    /// To learn more about KQL go to [https://docs.microsoft.com/en-us/azure/kusto/query/](https://docs.microsoft.com/en-us/azure/kusto/query)
    ///
    /// # Example
    /// ```no_run
    /// use azure_kusto_data::prelude::*;
    /// # #[tokio::main] async fn main() -> Result<(), Error> {
    /// use azure_kusto_data::client::QueryKind;
    /// use azure_kusto_data::request_options::RequestOptionsBuilder;
    ///
    /// let client = KustoClient::new(
    ///    ConnectionString::with_default_auth("https://mycluster.region.kusto.windows.net/"),
    ///    KustoClientOptions::default())?;
    ///    let result = client.execute_query_with_options(
    ///         "some_database",
    ///         "MyTable | take 10",
    ///         Some(RequestOptionsBuilder::default().with_request_app_name("app name").build().unwrap()))
    ///     .await?;
    ///
    ///   for table in result.into_primary_results() {
    ///        println!("{}", table.table_name);
    ///    }
    /// # Ok(())}
    /// ```
    ///
    #[must_use]
    pub fn execute_query_with_options(
        &self,
        database: impl Into<String>,
        query: impl Into<String>,
        options: impl Into<Option<RequestOptions>>,
    ) -> V2QueryRunner {
        V2QueryRunner(self.execute_with_options(database, query, QueryKind::Query, options))
    }

    /// Execute a KQL query.
    /// To learn more about KQL go to [https://docs.microsoft.com/en-us/azure/kusto/query/](https://docs.microsoft.com/en-us/azure/kusto/query)
    ///
    /// # Example
    /// ```no_run
    /// use azure_kusto_data::prelude::*;
    ///
    /// # #[tokio::main] async fn main() -> Result<(), Error> {
    /// let client = KustoClient::new(
    ///    ConnectionString::with_default_auth("https://mycluster.region.kusto.windows.net/"),
    ///    KustoClientOptions::default())?;
    ///
    ///    let result = client.execute_query("some_database", "MyTable | take 10").await?;
    ///
    ///    for table in result.into_primary_results() {
    ///        println!("{}", table.table_name);
    ///    }
    /// # Ok(())}
    /// ```
    #[must_use]
    pub fn execute_query(
        &self,
        database: impl Into<String>,
        query: impl Into<String>,
    ) -> V2QueryRunner {
        V2QueryRunner(self.execute_with_options(database, query, QueryKind::Query, None))
    }

    /// Execute a management command with additional options.
    /// To learn more about see [commands](https://docs.microsoft.com/en-us/azure/data-explorer/kusto/management/)
    ///
    /// # Example
    /// ```no_run
    /// use azure_kusto_data::prelude::*;
    /// # #[tokio::main] async fn main() -> Result<(), Error> {
    /// let client = KustoClient::new(
    ///    ConnectionString::with_default_auth("https://mycluster.region.kusto.windows.net/"),
    ///    KustoClientOptions::default())?;
    ///
    ///    let result = client.execute_command_with_options("some_database", ".show version",
    ///     Some(RequestOptionsBuilder::default().with_request_app_name("app name").build().unwrap()))
    ///     .await?;
    ///
    /// for table in result.tables {
    ///        println!("{}", table.table_name);
    ///    }
    /// # Ok(())}
    /// ```
    #[must_use]
    pub fn execute_command_with_options(
        &self,
        database: impl Into<String>,
        query: impl Into<String>,
        options: impl Into<Option<RequestOptions>>,
    ) -> V1QueryRunner {
        V1QueryRunner(self.execute_with_options(database, query, QueryKind::Management, options))
    }

    /// Execute a management command.
    /// To learn more about see [commands](https://docs.microsoft.com/en-us/azure/data-explorer/kusto/management/)
    ///
    /// # Example
    /// ```no_run
    /// use azure_kusto_data::prelude::*;
    ///
    /// # #[tokio::main] async fn main() -> Result<(), Error> {
    ///
    /// let client = KustoClient::new(
    ///    ConnectionString::with_default_auth("https://mycluster.region.kusto.windows.net/"),
    ///    KustoClientOptions::default())?;
    ///
    ///    let result = client.execute_command("some_database", ".show version").await?;
    ///
    ///    for table in result.tables {
    ///        println!("{}", table.table_name);
    ///    }
    /// # Ok(())}
    /// ```
    #[must_use]
    pub fn execute_command(
        &self,
        database: impl Into<String>,
        query: impl Into<String>,
    ) -> V1QueryRunner {
        V1QueryRunner(self.execute_with_options(database, query, QueryKind::Management, None))
    }
}

impl TryFrom<ConnectionString> for KustoClient {
    type Error = crate::error::Error;

    fn try_from(value: ConnectionString) -> Result<Self> {
        Self::new(value, KustoClientOptions::new())
    }
}