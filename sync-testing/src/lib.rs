//! Universal Sync Testing - test utilities and integration tests
//!
//! This crate provides test utilities for Universal Sync, importing both
//! proposer and acceptor functionality for integration testing.

pub mod yrs_crdt;

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

use iroh::{Endpoint, EndpointAddr, RelayMode};
use mls_rs::crypto::SignatureSecretKey;
use mls_rs::external_client::ExternalClient;
use mls_rs::identity::SigningIdentity;
use mls_rs::identity::basic::{BasicCredential, BasicIdentityProvider};
use mls_rs::{CipherSuite, CipherSuiteProvider, Client, CryptoProvider};
use mls_rs_crypto_rustcrypto::RustCryptoProvider;
use tempfile::TempDir;
use tracing_subscriber::{EnvFilter, fmt};
use universal_sync_acceptor::{AcceptorRegistry, SharedFjallStateStore, accept_connection};
use universal_sync_core::{
    ACCEPTOR_ADD_EXTENSION_TYPE, ACCEPTOR_REMOVE_EXTENSION_TYPE, ACCEPTORS_EXTENSION_TYPE,
    CRDT_REGISTRATION_EXTENSION_TYPE, CRDT_SNAPSHOT_EXTENSION_TYPE, MEMBER_ADDR_EXTENSION_TYPE,
    NoCrdtFactory, PAXOS_ALPN, SUPPORTED_CRDTS_EXTENSION_TYPE,
};
use universal_sync_proposer::{GroupClient, ReplContext};
pub use yrs_crdt::{YrsCrdt, YrsCrdtFactory};

/// Default cipher suite for testing
pub const TEST_CIPHER_SUITE: CipherSuite = CipherSuite::CURVE25519_AES128;

/// Create a test crypto provider
#[must_use]
pub fn test_crypto_provider() -> RustCryptoProvider {
    RustCryptoProvider::default()
}

/// Create a test cipher suite provider
///
/// # Panics
/// Panics if the cipher suite is not available.
#[must_use]
pub fn test_cipher_suite(
    crypto: &RustCryptoProvider,
) -> <RustCryptoProvider as CryptoProvider>::CipherSuiteProvider {
    crypto
        .cipher_suite_provider(TEST_CIPHER_SUITE)
        .expect("cipher suite should be available")
}

/// Create a basic identity provider for testing
#[must_use]
pub fn test_identity_provider() -> BasicIdentityProvider {
    BasicIdentityProvider::new()
}

/// Type alias for the cipher suite provider used in tests
pub type TestCipherSuiteProvider = <RustCryptoProvider as CryptoProvider>::CipherSuiteProvider;

/// Result of creating a test client
pub struct TestClientResult<C> {
    /// The MLS client
    pub client: Client<C>,
    /// The signing secret key
    pub signer: SignatureSecretKey,
    /// The cipher suite provider
    pub cipher_suite: TestCipherSuiteProvider,
}

/// Create a test MLS client with the given identity name
///
/// This returns a fully configured client that can create groups.
///
/// # Arguments
/// * `name` - A human-readable name for this client (e.g., "alice", "bob")
///
/// # Panics
/// Panics if key generation or client building fails.
#[must_use]
pub fn test_client(name: &str) -> TestClientResult<impl mls_rs::client_builder::MlsConfig> {
    let crypto = test_crypto_provider();
    let cipher_suite = test_cipher_suite(&crypto);

    // Generate a signing key pair
    let (secret_key, public_key) = cipher_suite
        .signature_key_generate()
        .expect("key generation should succeed");

    // Create a basic credential
    let credential = BasicCredential::new(name.as_bytes().to_vec());
    let signing_identity = SigningIdentity::new(credential.into_credential(), public_key);

    // Build the client with signing identity and custom extension types
    let client = Client::builder()
        .crypto_provider(crypto)
        .identity_provider(test_identity_provider())
        .signing_identity(signing_identity, secret_key.clone(), TEST_CIPHER_SUITE)
        .extension_type(ACCEPTORS_EXTENSION_TYPE)
        .extension_type(ACCEPTOR_ADD_EXTENSION_TYPE)
        .extension_type(ACCEPTOR_REMOVE_EXTENSION_TYPE)
        .extension_type(MEMBER_ADDR_EXTENSION_TYPE)
        .extension_type(SUPPORTED_CRDTS_EXTENSION_TYPE)
        .extension_type(CRDT_REGISTRATION_EXTENSION_TYPE)
        .extension_type(CRDT_SNAPSHOT_EXTENSION_TYPE)
        .build();

    TestClientResult {
        client,
        signer: secret_key,
        cipher_suite,
    }
}

/// Create a test [`GroupClient`] with the given identity name and endpoint.
///
/// This combines a fully configured MLS client with an iroh endpoint,
/// providing a convenient high-level API for creating and joining groups.
///
/// The client is pre-registered with a `NoCrdtFactory` for the "none" type.
///
/// # Arguments
/// * `name` - A human-readable name for this client (e.g., "alice", "bob")
/// * `endpoint` - The iroh endpoint for networking
///
/// # Panics
/// Panics if key generation or client building fails.
#[must_use]
pub fn test_group_client(
    name: &'static str,
    endpoint: iroh::Endpoint,
) -> GroupClient<impl mls_rs::client_builder::MlsConfig, TestCipherSuiteProvider> {
    let result = test_client(name);
    let mut client = GroupClient::new(result.client, result.signer, result.cipher_suite, endpoint);
    // Register default CRDT factory
    client.register_crdt_factory(NoCrdtFactory);
    client
}

/// Create a test [`ReplContext`] with the given identity name and endpoint.
///
/// This combines a fully configured MLS client with an iroh endpoint
/// and wraps it in a `ReplContext` for testing REPL commands.
///
/// The client is pre-registered with a `NoCrdtFactory` for the "none" type.
///
/// # Arguments
/// * `name` - A human-readable name for this client (e.g., "alice", "bob")
/// * `endpoint` - The iroh endpoint for networking
///
/// # Panics
/// Panics if key generation or client building fails.
#[must_use]
pub fn test_repl_context(
    name: &'static str,
    endpoint: iroh::Endpoint,
) -> ReplContext<impl mls_rs::client_builder::MlsConfig, TestCipherSuiteProvider> {
    let client = test_group_client(name, endpoint);
    ReplContext::new(client)
}

/// Create a test [`GroupClient`] with `YrsCrdtFactory` registered.
///
/// This is a convenience wrapper around [`test_group_client`] that also
/// registers the Yrs CRDT factory.
#[must_use]
pub fn test_yrs_group_client(
    name: &'static str,
    endpoint: Endpoint,
) -> GroupClient<impl mls_rs::client_builder::MlsConfig, TestCipherSuiteProvider> {
    let mut client = test_group_client(name, endpoint);
    client.register_crdt_factory(YrsCrdtFactory::new());
    client
}

// =============================================================================
// Test infrastructure helpers
// =============================================================================

/// Initialize tracing for tests. Safe to call multiple times.
pub fn init_tracing() {
    let _ = fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("universal_sync=debug")),
        )
        .with_test_writer()
        .try_init();
}

/// Create an iroh endpoint for testing.
///
/// Uses an empty builder with relays disabled and binds only to localhost
/// for faster local-only connections.
///
/// # Panics
/// Panics if the endpoint fails to bind.
pub async fn test_endpoint() -> Endpoint {
    let bind_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0));
    Endpoint::empty_builder(RelayMode::Disabled)
        .alpns(vec![PAXOS_ALPN.to_vec()])
        .bind_addr(bind_addr)
        .expect("valid bind address")
        .bind()
        .await
        .expect("failed to create endpoint")
}

/// Spawn a test acceptor. Returns the task handle, the acceptor's address,
/// and the temp directory (must be kept alive for the acceptor's state store).
///
/// # Panics
/// Panics if the acceptor fails to start.
pub async fn spawn_acceptor() -> (tokio::task::JoinHandle<()>, EndpointAddr, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let endpoint = test_endpoint().await;
    let addr = endpoint.addr();

    let crypto = test_crypto_provider();
    let cipher_suite = test_cipher_suite(&crypto);

    let state_store = SharedFjallStateStore::open(temp_dir.path())
        .await
        .expect("open state store");

    let external_client = ExternalClient::builder()
        .crypto_provider(crypto)
        .identity_provider(test_identity_provider())
        .build();

    let registry =
        AcceptorRegistry::new(external_client, cipher_suite, state_store, endpoint.clone());

    let task = tokio::spawn({
        let endpoint = endpoint.clone();
        async move {
            loop {
                if let Some(incoming) = endpoint.accept().await {
                    let registry = registry.clone();
                    tokio::spawn(async move {
                        let _ = accept_connection(incoming, registry).await;
                    });
                }
            }
        }
    });

    (task, addr, temp_dir)
}
