//! Editor client for creating and joining collaborative documents
//!
//! This module provides [`EditorClient`], a high-level API for
//! creating and joining synchronized documents.

use error_stack::Report;
use iroh::{Endpoint, EndpointAddr};
use mls_rs::client_builder::MlsConfig;
use mls_rs::identity::basic::{BasicCredential, BasicIdentityProvider};
use mls_rs::identity::SigningIdentity;
use mls_rs::{CipherSuite, CipherSuiteProvider, Client, CryptoProvider, MlsMessage};
use mls_rs_crypto_rustcrypto::RustCryptoProvider;
use sha2::{Digest, Sha256};
use universal_sync_core::{
    NoCrdtFactory, ACCEPTOR_ADD_EXTENSION_TYPE, ACCEPTOR_REMOVE_EXTENSION_TYPE,
    ACCEPTORS_EXTENSION_TYPE, CRDT_REGISTRATION_EXTENSION_TYPE, MEMBER_ADDR_EXTENSION_TYPE,
    SUPPORTED_CRDTS_EXTENSION_TYPE,
};
use universal_sync_proposer::{GroupClient, GroupError};
use universal_sync_testing::YrsCrdtFactory;

use crate::SyncedDocument;

/// Default cipher suite
const DEFAULT_CIPHER_SUITE: CipherSuite = CipherSuite::CURVE25519_AES128;

/// A client for creating and joining collaborative documents.
///
/// This wraps a `GroupClient` and provides document-oriented APIs.
pub struct EditorClient<C, CS>
where
    C: MlsConfig + Clone + Send + Sync + 'static,
    CS: CipherSuiteProvider + Clone + Send + Sync + 'static,
{
    /// The underlying group client
    client: GroupClient<C, CS>,
}

/// Derive a yrs client ID from a signing public key using SHA256.
fn derive_yrs_client_id(signing_key: &[u8]) -> u64 {
    let hash = Sha256::digest(signing_key);
    u64::from_be_bytes(hash[..8].try_into().expect("sha256 produces 32 bytes"))
}

/// Create a new editor client with the given name and endpoint.
///
/// This is a convenience function that creates a fully configured
/// editor client with default settings.
///
/// # Panics
/// Panics if key generation fails.
#[must_use]
pub fn create_editor_client(
    name: &str,
    endpoint: Endpoint,
) -> EditorClient<impl MlsConfig, impl CipherSuiteProvider + Clone> {
    let crypto = RustCryptoProvider::default();
    let cipher_suite = crypto
        .cipher_suite_provider(DEFAULT_CIPHER_SUITE)
        .expect("cipher suite should be available");

    // Generate a signing key pair
    let (secret_key, public_key) = cipher_suite
        .signature_key_generate()
        .expect("key generation should succeed");

    let signing_public_key = public_key.as_ref().to_vec();

    // Create a basic credential
    let credential = BasicCredential::new(name.as_bytes().to_vec());
    let signing_identity = SigningIdentity::new(credential.into_credential(), public_key);

    // Build the client with signing identity and custom extension types
    let client = Client::builder()
        .crypto_provider(crypto)
        .identity_provider(BasicIdentityProvider::new())
        .signing_identity(signing_identity, secret_key.clone(), DEFAULT_CIPHER_SUITE)
        .extension_type(ACCEPTORS_EXTENSION_TYPE)
        .extension_type(ACCEPTOR_ADD_EXTENSION_TYPE)
        .extension_type(ACCEPTOR_REMOVE_EXTENSION_TYPE)
        .extension_type(MEMBER_ADDR_EXTENSION_TYPE)
        .extension_type(SUPPORTED_CRDTS_EXTENSION_TYPE)
        .extension_type(CRDT_REGISTRATION_EXTENSION_TYPE)
        .build();

    // Create yrs factory with derived client ID
    let yrs_client_id = derive_yrs_client_id(&signing_public_key);
    tracing::info!(yrs_client_id, "Generated yrs client ID from signing key");
    let yrs_factory = YrsCrdtFactory::with_fixed_client_id(yrs_client_id);

    let mut group_client = GroupClient::new(client, secret_key, cipher_suite, endpoint);

    // Register CRDT factories
    group_client.register_crdt_factory(NoCrdtFactory);
    group_client.register_crdt_factory(yrs_factory);

    EditorClient {
        client: group_client,
    }
}

impl<C, CS> EditorClient<C, CS>
where
    C: MlsConfig + Clone + Send + Sync + 'static,
    CS: CipherSuiteProvider + Clone + Send + Sync + 'static,
{
    /// Create a new editor client from an existing group client.
    ///
    /// This registers the yrs CRDT factory automatically.
    ///
    /// # Arguments
    /// * `client` - The underlying group client
    /// * `signing_public_key` - The MLS signing public key (for yrs client ID)
    #[must_use]
    pub fn new(mut client: GroupClient<C, CS>, signing_public_key: &[u8]) -> Self {
        // Register CRDT factories
        let yrs_client_id = derive_yrs_client_id(signing_public_key);
        let yrs_factory = YrsCrdtFactory::with_fixed_client_id(yrs_client_id);
        client.register_crdt_factory(NoCrdtFactory);
        client.register_crdt_factory(yrs_factory);

        Self { client }
    }

    /// Create an editor client from parts without registering CRDT factories.
    ///
    /// Use this when you need more control over factory registration.
    #[must_use]
    pub fn from_parts(client: GroupClient<C, CS>) -> Self {
        Self { client }
    }

    /// Create a new collaborative document.
    ///
    /// # Arguments
    /// * `acceptors` - Endpoint addresses of acceptors (can be empty for local-only)
    ///
    /// # Errors
    /// Returns an error if group creation fails.
    pub async fn create_document(
        &self,
        acceptors: &[EndpointAddr],
    ) -> Result<SyncedDocument<C, CS>, Report<GroupError>> {
        let group = self.client.create_group(acceptors, "yrs").await?;
        Ok(SyncedDocument::new(group))
    }

    /// Join an existing document using a welcome message.
    ///
    /// # Arguments
    /// * `welcome_bytes` - The welcome message received from the inviter
    ///
    /// # Errors
    /// Returns an error if joining fails.
    pub async fn join_document(
        &self,
        welcome_bytes: &[u8],
    ) -> Result<SyncedDocument<C, CS>, Report<GroupError>> {
        // The Group's internal CRDT is automatically initialized from the
        // welcome bundle's CRDT snapshot, so we just need to wrap the group.
        let group = self.client.join_group(welcome_bytes).await?;
        Ok(SyncedDocument::new(group))
    }

    /// Generate a key package for joining a document.
    ///
    /// # Errors
    /// Returns an error if key package generation fails.
    pub fn generate_key_package(&self) -> Result<MlsMessage, Report<GroupError>> {
        self.client.generate_key_package()
    }

    /// Wait for an incoming welcome message.
    ///
    /// Returns `None` if the client is shutting down.
    pub async fn recv_welcome(&mut self) -> Option<Vec<u8>> {
        self.client.recv_welcome().await
    }

    /// Try to receive a welcome message without blocking.
    pub fn try_recv_welcome(&mut self) -> Option<Vec<u8>> {
        self.client.try_recv_welcome()
    }

    /// Get a reference to the underlying group client.
    pub fn group_client(&self) -> &GroupClient<C, CS> {
        &self.client
    }
}

/// Get a document from a group.
///
/// This is a helper for getting a `SyncedDocument` from an existing `Group`.
#[must_use]
pub fn document_from_group<C, CS>(
    group: universal_sync_proposer::Group<C, CS>,
) -> SyncedDocument<C, CS>
where
    C: MlsConfig + Clone + Send + Sync + 'static,
    CS: CipherSuiteProvider + Clone + Send + Sync + 'static,
{
    SyncedDocument::new(group)
}
