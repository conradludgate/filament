//! Group registry implementation
//!
//! Provides a concrete implementation of [`GroupRegistry`] for managing
//! multiple groups on an acceptor server.

use std::sync::Arc;

use mls_rs::external_client::ExternalClient;
use mls_rs::external_client::builder::MlsConfig as ExternalMlsConfig;
use mls_rs::{CipherSuiteProvider, MlsMessage};

use crate::acceptor::GroupAcceptor;
use crate::handshake::GroupId;
use crate::server::GroupRegistry;
use crate::state_store::{GroupStateStore, SharedFjallStateStore};

/// Concrete implementation of [`GroupRegistry`]
///
/// Manages multiple MLS groups for an acceptor server. Each group is
/// identified by its 32-byte [`GroupId`].
///
/// Groups are persisted to the state store and reconstructed on each
/// access. This ensures groups survive server restarts.
///
/// # Type Parameters
/// * `C` - The MLS external client config type
/// * `CS` - The cipher suite provider type
#[derive(Clone)]
pub struct AcceptorRegistry<C, CS>
where
    C: ExternalMlsConfig + Clone,
    CS: CipherSuiteProvider + Clone,
{
    /// The external client for creating new groups (shared)
    external_client: Arc<ExternalClient<C>>,

    /// Cipher suite provider (cloned for each group)
    cipher_suite: CS,

    /// Persistent state store (shared across all groups)
    state_store: SharedFjallStateStore,
}

impl<C, CS> AcceptorRegistry<C, CS>
where
    C: ExternalMlsConfig + Clone,
    CS: CipherSuiteProvider + Clone,
{
    /// Create a new registry
    ///
    /// # Arguments
    /// * `external_client` - MLS external client for joining groups
    /// * `cipher_suite` - Cipher suite provider for signature verification
    /// * `state_store` - Persistent state store for all groups
    pub fn new(
        external_client: ExternalClient<C>,
        cipher_suite: CS,
        state_store: SharedFjallStateStore,
    ) -> Self {
        Self {
            external_client: Arc::new(external_client),
            cipher_suite,
            state_store,
        }
    }

    /// Get the state store
    pub fn state_store(&self) -> &SharedFjallStateStore {
        &self.state_store
    }

    /// List all registered groups
    pub fn list_groups(&self) -> Vec<GroupId> {
        self.state_store.list_groups()
    }

    /// Create an acceptor from stored GroupInfo bytes
    fn create_acceptor_from_bytes(
        &self,
        group_info_bytes: &[u8],
    ) -> Result<GroupAcceptor<C, CS>, String> {
        let mls_message = MlsMessage::from_bytes(group_info_bytes)
            .map_err(|e| format!("failed to parse MLS message: {e}"))?;

        let external_group = self
            .external_client
            .observe_group(mls_message, None)
            .map_err(|e| format!("failed to observe group: {e}"))?;

        Ok(GroupAcceptor::new(
            external_group,
            self.cipher_suite.clone(),
        ))
    }
}

impl<C, CS> GroupRegistry for AcceptorRegistry<C, CS>
where
    C: ExternalMlsConfig + Clone + Send + Sync + 'static,
    CS: CipherSuiteProvider + Clone + Send + Sync + 'static,
{
    type Acceptor = GroupAcceptor<C, CS>;
    type StateStore = GroupStateStore;

    fn get_group(&self, group_id: &GroupId) -> Option<(Self::Acceptor, Self::StateStore)> {
        // Load GroupInfo from persistent storage
        let group_info_bytes = self.state_store.get_group_info(group_id)?;

        // Recreate the acceptor from stored GroupInfo
        let acceptor = self.create_acceptor_from_bytes(&group_info_bytes).ok()?;

        let state = self.state_store.for_group(*group_id);

        Some((acceptor, state))
    }

    fn create_group(
        &self,
        group_info_bytes: &[u8],
    ) -> Result<(GroupId, Self::Acceptor, Self::StateStore), String> {
        // Parse and observe the group
        let mls_message = MlsMessage::from_bytes(group_info_bytes)
            .map_err(|e| format!("failed to parse MLS message: {e}"))?;

        let external_group = self
            .external_client
            .observe_group(mls_message, None)
            .map_err(|e| format!("failed to observe group: {e}"))?;

        // Extract the group ID
        let mls_group_id = external_group.group_context().group_id.clone();
        let group_id = GroupId::from_slice(&mls_group_id);

        // Create the acceptor
        let acceptor = GroupAcceptor::new(external_group, self.cipher_suite.clone());

        // Persist the GroupInfo bytes
        self.state_store
            .store_group(&group_id, group_info_bytes)
            .map_err(|e| format!("failed to persist group: {e}"))?;

        let state = self.state_store.for_group(group_id);

        Ok((group_id, acceptor, state))
    }
}
