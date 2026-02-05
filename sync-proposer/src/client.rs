//! `GroupClient` - high-level client abstraction for creating and joining groups.
//!
//! This module provides [`GroupClient`], which combines an MLS client, iroh endpoint,
//! and connection management into a single convenient type.

use std::sync::Arc;

use error_stack::Report;
use iroh::{Endpoint, EndpointAddr};
use mls_rs::client_builder::MlsConfig;
use mls_rs::crypto::SignatureSecretKey;
use mls_rs::{CipherSuiteProvider, Client, ExtensionList, MlsMessage};

use crate::MemberAddrExt;
use crate::connection::ConnectionManager;
use crate::error::GroupError;
use crate::group::Group;

/// A high-level client for creating and joining synchronized groups.
///
/// `GroupClient` combines:
/// - An MLS [`Client`] for cryptographic group operations
/// - A [`ConnectionManager`] for peer-to-peer networking
/// - A signing key for authentication
/// - A cipher suite for cryptographic operations
///
/// # Example
///
/// ```ignore
/// use universal_sync_proposer::GroupClient;
///
/// // Create a client
/// let group_client = GroupClient::new(mls_client, signer, cipher_suite, endpoint);
///
/// // Create a new group
/// let group = group_client.create_group(&[acceptor_addr]).await?;
///
/// // Generate a key package to share with others
/// let key_package = group_client.generate_key_package()?;
/// ```
pub struct GroupClient<C, CS> {
    /// The MLS client
    client: Arc<Client<C>>,
    /// The signing secret key
    signer: SignatureSecretKey,
    /// The cipher suite provider
    cipher_suite: CS,
    /// The connection manager for p2p connections
    connection_manager: ConnectionManager,
}

impl<C, CS> GroupClient<C, CS>
where
    C: MlsConfig + Clone + Send + Sync + 'static,
    CS: CipherSuiteProvider + Clone + Send + Sync + 'static,
{
    /// Create a new `GroupClient`.
    ///
    /// # Arguments
    /// * `client` - The MLS client
    /// * `signer` - The signing secret key for this client
    /// * `cipher_suite` - The cipher suite provider
    /// * `endpoint` - The iroh endpoint for networking
    pub fn new(
        client: Client<C>,
        signer: SignatureSecretKey,
        cipher_suite: CS,
        endpoint: Endpoint,
    ) -> Self {
        Self {
            client: Arc::new(client),
            signer,
            cipher_suite,
            connection_manager: ConnectionManager::new(endpoint),
        }
    }

    /// Get this client's endpoint address.
    ///
    /// This is the address other peers use to connect to this client.
    #[must_use]
    pub fn addr(&self) -> EndpointAddr {
        self.connection_manager.endpoint().addr()
    }

    /// Get a reference to the iroh endpoint.
    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        self.connection_manager.endpoint()
    }

    /// Get a reference to the connection manager.
    #[must_use]
    pub fn connection_manager(&self) -> &ConnectionManager {
        &self.connection_manager
    }

    /// Generate a key package for joining a group.
    ///
    /// The key package includes this client's endpoint address as an extension,
    /// which allows the inviter to send the welcome message directly.
    ///
    /// # Errors
    /// Returns an error if key package generation fails.
    pub fn generate_key_package(&self) -> Result<MlsMessage, Report<GroupError>> {
        // Include our endpoint address in the key package
        let member_addr_ext = MemberAddrExt::new(self.connection_manager.endpoint().addr());
        let mut kp_extensions = ExtensionList::default();
        kp_extensions.set_from(member_addr_ext).map_err(|e| {
            Report::new(GroupError).attach(format!("failed to set member address extension: {e:?}"))
        })?;

        self.client
            .generate_key_package_message(kp_extensions, ExtensionList::default(), None)
            .map_err(|e| {
                Report::new(GroupError).attach(format!("failed to generate key package: {e:?}"))
            })
    }

    /// Create a new group, optionally registering with acceptors.
    ///
    /// # Arguments
    /// * `acceptors` - Endpoint addresses of acceptors to register with (can be empty)
    ///
    /// # Errors
    /// Returns an error if group creation or acceptor registration fails.
    pub async fn create_group(
        &self,
        acceptors: &[EndpointAddr],
    ) -> Result<Group<C, CS>, Report<GroupError>> {
        Group::create(
            &self.client,
            self.signer.clone(),
            self.cipher_suite.clone(),
            &self.connection_manager,
            acceptors,
            None, // No CRDT - use with_crdt variant for CRDT support
        )
        .await
    }

    /// Join an existing group using a welcome message.
    ///
    /// # Arguments
    /// * `welcome_bytes` - The serialized welcome message received from the inviter
    ///
    /// # Errors
    /// Returns an error if joining the group fails.
    pub async fn join_group(
        &self,
        welcome_bytes: &[u8],
    ) -> Result<Group<C, CS>, Report<GroupError>> {
        Group::join(
            &self.client,
            self.signer.clone(),
            self.cipher_suite.clone(),
            &self.connection_manager,
            welcome_bytes,
            None, // No CRDT - use with_crdt variant for CRDT support
        )
        .await
    }

    /// Wait for a welcome message from an inviter.
    ///
    /// This listens for an incoming connection with a `SendWelcome` handshake
    /// and returns the welcome bytes.
    ///
    /// # Errors
    /// Returns an error if receiving the welcome fails.
    pub async fn wait_for_welcome(&self) -> Result<Vec<u8>, Report<GroupError>> {
        crate::group::wait_for_welcome(self.connection_manager.endpoint()).await
    }
}

impl<C, CS> Clone for GroupClient<C, CS>
where
    CS: Clone,
{
    fn clone(&self) -> Self {
        Self {
            client: Arc::clone(&self.client),
            signer: self.signer.clone(),
            cipher_suite: self.cipher_suite.clone(),
            connection_manager: self.connection_manager.clone(),
        }
    }
}
