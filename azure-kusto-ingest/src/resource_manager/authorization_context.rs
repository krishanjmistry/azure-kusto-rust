use azure_kusto_data::prelude::KustoClient;
use serde_json::Value;

use super::cache::ThreadSafeCachedValue;
use super::utils::get_column_index;
use super::RESOURCE_REFRESH_PERIOD;

pub(crate) type KustoIdentityToken = String;

const AUTHORIZATION_CONTEXT: &str = "AuthorizationContext";

#[derive(thiserror::Error, Debug)]
pub enum KustoIdentityTokenError {
    #[error("Kusto expected 1 table in results, found {0}")]
    ExpectedOneTable(usize),

    #[error("Kusto expected 1 row in table, found {0}")]
    ExpectedOneRow(usize),

    #[error("Column {0} not found in table")]
    ColumnNotFound(String),

    #[error("Invalid JSON response from Kusto: {0:?}")]
    InvalidJSONResponse(Value),

    #[error("Token is empty")]
    EmptyToken,

    #[error(transparent)]
    KustoError(#[from] azure_kusto_data::error::Error),
}

type Result<T> = std::result::Result<T, KustoIdentityTokenError>;
/// Logic to obtain a Kusto identity token from the management endpoint. This auth token is a temporary token
#[derive(Debug, Clone)]
pub(crate) struct AuthorizationContext {
    /// A client against a Kusto ingestion cluster
    client: KustoClient,
    /// Cache of the Kusto identity token
    token_cache: ThreadSafeCachedValue<KustoIdentityToken>,
}

impl AuthorizationContext {
    pub fn new(client: KustoClient) -> Self {
        Self {
            client,
            token_cache: ThreadSafeCachedValue::new(RESOURCE_REFRESH_PERIOD),
        }
    }

    /// Executes a KQL query to get the Kusto identity token from the management endpoint
    async fn query_kusto_identity_token(&self) -> Result<KustoIdentityToken> {
        let results = self
            .client
            .execute_command("NetDefaultDB", ".get kusto identity token", None)
            .await?;

        // Check that there is only 1 table in the results returned by the query
        let table = match &results.tables[..] {
            [a] => a,
            _ => {
                return Err(KustoIdentityTokenError::ExpectedOneTable(
                    results.tables.len(),
                ))
            }
        };

        // Check that a column in this table actually exists called `AuthorizationContext`
        let index = get_column_index(table, AUTHORIZATION_CONTEXT).ok_or(
            KustoIdentityTokenError::ColumnNotFound(AUTHORIZATION_CONTEXT.into()),
        )?;

        // Check that there is only 1 row in the table, and that the value in the first row at the given index is not empty
        let token = match &table.rows[..] {
            [row] => row
                .get(index)
                .ok_or(KustoIdentityTokenError::ColumnNotFound(
                    AUTHORIZATION_CONTEXT.into(),
                ))?,
            _ => return Err(KustoIdentityTokenError::ExpectedOneRow(table.rows.len())),
        };

        // Convert the JSON string into a Rust string
        let token = token
            .as_str()
            .ok_or(KustoIdentityTokenError::InvalidJSONResponse(
                token.to_owned(),
            ))?;

        if token.chars().all(char::is_whitespace) {
            return Err(KustoIdentityTokenError::EmptyToken);
        }

        Ok(token.to_string())
    }

    /// Fetches the latest Kusto identity token, either retrieving from cache if valid, or by executing a KQL query
    pub(crate) async fn get(&self) -> Result<KustoIdentityToken> {
        self.token_cache
            .get(self.query_kusto_identity_token())
            .await
    }
}
