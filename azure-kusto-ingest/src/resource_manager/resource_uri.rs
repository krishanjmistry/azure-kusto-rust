use azure_core::ClientOptions;
use azure_storage::StorageCredentials;
use azure_storage_blobs::prelude::{ClientBuilder, ContainerClient};
use azure_storage_queues::{QueueClient, QueueServiceClientBuilder};
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum ResourceUriError {
    #[error("URI scheme must be 'https', was '{0}'")]
    InvalidScheme(String),

    #[error("URI host must be a domain")]
    InvalidHost,

    #[error("Object name is missing in the URI")]
    MissingObjectName,

    #[error("SAS token is missing in the URI as a query parameter")]
    MissingSasToken,

    #[error("Account name is missing in the URI")]
    MissingAccountName,

    #[error(transparent)]
    ParseError(#[from] url::ParseError),

    #[error(transparent)]
    AzureError(#[from] azure_core::Error),
}

/// Parsing logic of resource URIs as returned by the Kusto management endpoint
#[derive(Debug, Clone)]
pub(crate) struct ResourceUri {
    pub(crate) service_uri: String,
    pub(crate) object_name: String,
    pub(crate) account_name: String,
    pub(crate) sas_token: StorageCredentials,
}

impl TryFrom<&str> for ResourceUri {
    type Error = ResourceUriError;

    fn try_from(uri: &str) -> Result<Self, Self::Error> {
        let parsed_uri = Url::parse(uri)?;

        match parsed_uri.scheme() {
            "https" => {}
            other_scheme => return Err(ResourceUriError::InvalidScheme(other_scheme.to_string())),
        };

        let host_string = match parsed_uri.host() {
            Some(url::Host::Domain(host_string)) => host_string,
            _ => return Err(ResourceUriError::InvalidHost),
        };

        let service_uri = String::from("https://") + host_string;

        // WIBNI: better parsing that this conforms to a storage resource URI,
        // perhaps then ResourceUri could take a type like ResourceUri<Queue> or ResourceUri<Container>
        let (account_name, _service_endpoint) = host_string
            .split_once('.')
            .ok_or(ResourceUriError::MissingAccountName)?;

        let object_name = match parsed_uri.path_segments() {
            Some(mut path_segments) => {
                let object_name = match path_segments.next() {
                    Some(object_name) if !object_name.is_empty() => object_name,
                    _ => return Err(ResourceUriError::MissingObjectName),
                };
                // Ensure there is only one path segment (i.e. the object name)
                if path_segments.next().is_some() {
                    return Err(ResourceUriError::MissingObjectName);
                };
                object_name
            }
            None => return Err(ResourceUriError::MissingObjectName),
        };

        let sas_token = parsed_uri
            .query()
            .ok_or(ResourceUriError::MissingSasToken)?;

        let sas_token = StorageCredentials::sas_token(sas_token)?;

        Ok(Self {
            service_uri,
            object_name: object_name.to_string(),
            account_name: account_name.to_string(),
            sas_token,
        })
    }
}

/// Trait to be used to create an Azure client from a resource URI with configurability of ClientOptions
pub(crate) trait ClientFromResourceUri {
    fn create_client(resource_uri: ResourceUri, client_options: ClientOptions) -> Self;
}

impl ClientFromResourceUri for QueueClient {
    fn create_client(resource_uri: ResourceUri, client_options: ClientOptions) -> Self {
        QueueServiceClientBuilder::with_location(
            azure_storage::CloudLocation::Custom {
                uri: resource_uri.service_uri,
                account: resource_uri.account_name,
            },
            resource_uri.sas_token,
        )
        .client_options(client_options)
        .build()
        .queue_client(resource_uri.object_name)
    }
}

impl ClientFromResourceUri for ContainerClient {
    fn create_client(resource_uri: ResourceUri, client_options: ClientOptions) -> Self {
        ClientBuilder::with_location(
            azure_storage::CloudLocation::Custom {
                uri: resource_uri.service_uri,
                account: resource_uri.account_name,
            },
            resource_uri.sas_token,
        )
        .client_options(client_options)
        .container_client(resource_uri.object_name)
    }
}

#[cfg(test)]
mod tests {
    use azure_storage::StorageCredentialsInner;

    use super::*;
    use std::convert::TryFrom;

    #[test]
    fn resource_uri_try_from() {
        let uri = "https://storageaccountname.blob.core.windows.com/containerobjectname?sas=token";
        let resource_uri = ResourceUri::try_from(uri).unwrap();

        assert_eq!(
            resource_uri.service_uri,
            "https://storageaccountname.blob.core.windows.com"
        );
        assert_eq!(resource_uri.object_name, "containerobjectname");

        let storage_credential_inner = std::sync::Arc::into_inner(resource_uri.sas_token.0)
            .unwrap()
            .into_inner();
        assert!(matches!(
            storage_credential_inner,
            StorageCredentialsInner::SASToken(_)
        ));

        if let StorageCredentialsInner::SASToken(sas_vec) = storage_credential_inner {
            assert_eq!(sas_vec.len(), 1);
            assert_eq!(sas_vec[0].0, "sas");
            assert_eq!(sas_vec[0].1, "token");
        }
    }

    #[test]
    fn invalid_scheme() {
        let uri = "http://storageaccountname.blob.core.windows.com/containerobjectname?sas=token";
        let resource_uri = ResourceUri::try_from(uri);

        assert!(resource_uri.is_err());
        assert!(matches!(
            resource_uri.unwrap_err(),
            ResourceUriError::InvalidScheme(_)
        ));
    }

    #[test]
    fn missing_host_str() {
        let uri = "https:";
        let resource_uri = ResourceUri::try_from(uri);
        println!("{:#?}", resource_uri);

        assert!(resource_uri.is_err());
        assert!(matches!(
            resource_uri.unwrap_err(),
            ResourceUriError::ParseError(_)
        ));
    }

    #[test]
    fn invalid_host_ipv4() {
        let uri = "https://127.0.0.1/containerobjectname?sas=token";
        let resource_uri = ResourceUri::try_from(uri);

        assert!(resource_uri.is_err());
        assert!(matches!(
            resource_uri.unwrap_err(),
            ResourceUriError::InvalidHost
        ));
    }

    #[test]
    fn invalid_host_ipv6() {
        let uri = "https://[3FFE:FFFF:0::CD30]/containerobjectname?sas=token";
        let resource_uri = ResourceUri::try_from(uri);
        println!("{:#?}", resource_uri);

        assert!(resource_uri.is_err());
        assert!(matches!(
            resource_uri.unwrap_err(),
            ResourceUriError::InvalidHost
        ));
    }

    #[test]
    fn missing_object_name() {
        let uri = "https://storageaccountname.blob.core.windows.com/?sas=token";
        let resource_uri = ResourceUri::try_from(uri);
        println!("{:#?}", resource_uri);

        assert!(resource_uri.is_err());
        assert!(matches!(
            resource_uri.unwrap_err(),
            ResourceUriError::MissingObjectName
        ));
    }

    #[test]
    fn missing_sas_token() {
        let uri = "https://storageaccountname.blob.core.windows.com/containerobjectname";
        let resource_uri = ResourceUri::try_from(uri);
        println!("{:#?}", resource_uri);

        assert!(resource_uri.is_err());
        assert!(matches!(
            resource_uri.unwrap_err(),
            ResourceUriError::MissingSasToken
        ));
    }

    #[test]
    fn queue_client_from_resource_uri() {
        let resource_uri = ResourceUri {
            service_uri: "https://mystorageaccount.queue.core.windows.net".to_string(),
            object_name: "queuename".to_string(),
            account_name: "mystorageaccount".to_string(),
            sas_token: StorageCredentials::sas_token("sas=token").unwrap(),
        };

        let client_options = ClientOptions::default();
        let queue_client = QueueClient::create_client(resource_uri, client_options);

        assert_eq!(queue_client.queue_name(), "queuename");
    }

    #[test]
    fn container_client_from_resource_uri() {
        let resource_uri = ResourceUri {
            service_uri: "https://mystorageaccount.blob.core.windows.net".to_string(),
            object_name: "containername".to_string(),
            account_name: "mystorageaccount".to_string(),
            sas_token: StorageCredentials::sas_token("sas=token").unwrap(),
        };

        let client_options = ClientOptions::default();
        let container_client = ContainerClient::create_client(resource_uri, client_options);

        assert_eq!(container_client.container_name(), "containername");
    }
}
