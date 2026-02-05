//! Group registry implementation
//!
//! Provides [`AcceptorRegistry`] for managing multiple groups on an acceptor server.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use iroh::{Endpoint, EndpointAddr};
use mls_rs::external_client::builder::MlsConfig as ExternalMlsConfig;
use mls_rs::external_client::{ExternalClient, ExternalGroup};
use mls_rs::mls_rs_codec::MlsDecode;
use mls_rs::{CipherSuiteProvider, ExtensionList, MlsMessage};
use tokio::sync::watch;
use universal_sync_core::{ACCEPTORS_EXTENSION_TYPE, AcceptorId, AcceptorsExt, Epoch, GroupId};
use universal_sync_paxos::Learner;

use crate::acceptor::GroupAcceptor;
use crate::learner::GroupLearningActor;
use crate::state_store::{GroupStateStore, SharedFjallStateStore};

/// Return type for [`AcceptorRegistry::get_epoch_watcher`].
pub type EpochWatcher = (watch::Receiver<Epoch>, Box<dyn Fn() -> Epoch + Send>);

/// Epoch watcher for a group.
///
/// Contains a watch sender for epoch change notifications.
struct GroupEpochWatcher {
    tx: watch::Sender<Epoch>,
    rx: watch::Receiver<Epoch>,
}

impl GroupEpochWatcher {
    fn new(initial_epoch: Epoch) -> Self {
        let (tx, rx) = watch::channel(initial_epoch);
        Self { tx, rx }
    }

    fn notify(&self, epoch: Epoch) {
        let _ = self.tx.send(epoch);
    }
}

/// Manages multiple MLS groups for an acceptor server.
///
/// Each group is identified by its 32-byte [`GroupId`].
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

    /// Iroh endpoint for peer connections
    endpoint: Endpoint,

    /// Epoch watchers per group (also tracks which groups have learning actors)
    epoch_watchers: Arc<RwLock<HashMap<GroupId, Arc<GroupEpochWatcher>>>>,
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
    /// * `secret_key` - This acceptor's iroh secret key
    /// * `endpoint` - Iroh endpoint for peer connections
    pub fn new(
        external_client: ExternalClient<C>,
        cipher_suite: CS,
        state_store: SharedFjallStateStore,
        endpoint: Endpoint,
    ) -> Self {
        Self {
            external_client: Arc::new(external_client),
            cipher_suite,
            state_store,
            endpoint,
            epoch_watchers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get this acceptor's own ID (derived from public key)
    fn own_id(&self) -> AcceptorId {
        AcceptorId::from_bytes(*self.endpoint.id().as_bytes())
    }

    // /// Get this acceptor's own ID (derived from secret key)
    // pub(crate) fn own_id(&self) -> AcceptorId {
    //     AcceptorId::from_bytes(*self.secret_key.public().as_bytes())
    // }

    // /// Get the state store
    // pub(crate) fn state_store(&self) -> &SharedFjallStateStore {
    //     &self.state_store
    // }

    // /// List all registered groups
    // pub(crate) fn list_groups(&self) -> Vec<GroupId> {
    //     self.state_store.list_groups()
    // }

    /// Extract acceptor addresses from an extension list
    fn extract_acceptors_from_extensions(extensions: &ExtensionList) -> Vec<EndpointAddr> {
        for ext in extensions.iter() {
            if ext.extension_type == ACCEPTORS_EXTENSION_TYPE
                && let Ok(acceptors_ext) =
                    AcceptorsExt::mls_decode(&mut ext.extension_data.as_slice())
            {
                return acceptors_ext.acceptors().to_vec();
            }
        }
        vec![]
    }

    /// Extract acceptor addresses from an external group's context extensions
    fn extract_acceptors_from_group(external_group: &ExternalGroup<C>) -> Vec<EndpointAddr> {
        // Try group context extensions first
        let ctx_acceptors =
            Self::extract_acceptors_from_extensions(&external_group.group_context().extensions);
        if !ctx_acceptors.is_empty() {
            return ctx_acceptors;
        }

        // Fall back to group info extensions if available
        // Note: For newly created groups, acceptors are typically in GroupInfo extensions
        // set via set_group_info_ext(). When the acceptor observes the group, these
        // may not be directly accessible, so we rely on the creator setting them
        // in the group context extensions as well, or storing them separately.
        vec![]
    }

    /// Create an acceptor from stored `GroupInfo` bytes
    fn create_acceptor_from_bytes(
        &self,
        group_info_bytes: &[u8],
    ) -> Result<GroupAcceptor<C, CS>, String> {
        let mls_message = MlsMessage::from_bytes(group_info_bytes)
            .map_err(|e| format!("failed to parse MLS message: {e}"))?;

        let external_group = self
            .external_client
            .observe_group(mls_message, None, None)
            .map_err(|e| format!("failed to observe group: {e}"))?;

        // Extract acceptors from the group's extensions
        let acceptors = Self::extract_acceptors_from_group(&external_group);

        Ok(GroupAcceptor::new(
            external_group,
            self.cipher_suite.clone(),
            self.endpoint.secret_key().clone(),
            acceptors,
        ))
    }
}

impl<C, CS> AcceptorRegistry<C, CS>
where
    C: ExternalMlsConfig + Clone + Send + Sync + 'static,
    CS: CipherSuiteProvider + Clone + Send + Sync + 'static,
{
    /// Look up an existing group by ID
    ///
    /// Returns the acceptor and state store if the group exists.
    pub fn get_group(&self, group_id: &GroupId) -> Option<(GroupAcceptor<C, CS>, GroupStateStore)> {
        // Load GroupInfo from persistent storage
        let group_info_bytes = self.state_store.get_group_info(group_id)?;

        // Recreate the acceptor from stored GroupInfo
        let acceptor = self.create_acceptor_from_bytes(&group_info_bytes).ok()?;

        let state = self.state_store.for_group(*group_id);

        // Attach state store for epoch roster lookups during validation
        let mut acceptor = acceptor.with_state_store(state.clone());

        // Store the initial epoch roster for signature validation
        // (This may be redundant if we already stored it, but it's idempotent)
        acceptor.store_initial_epoch_roster();

        // Replay all accepted messages to bring the acceptor up to date
        // This is necessary because the GroupInfo we stored is from epoch 0,
        // but the acceptor may have processed commits since then.
        let historical = state.get_accepted_from(acceptor.current_round());

        for (proposal, message) in historical {
            tracing::debug!(epoch = ?proposal.epoch, "replaying commit for acceptor");
            // Apply the message to catch up the MLS state
            // Note: We ignore errors here since the message was already accepted
            if let Err(e) =
                futures::executor::block_on(Learner::apply(&mut acceptor, proposal, message))
            {
                tracing::warn!(?e, "failed to replay message");
            }
        }

        // Ensure epoch watcher and learning actor exist
        self.ensure_epoch_watcher_and_learning(
            *group_id,
            acceptor.current_round(),
            &acceptor,
            state.clone(),
        );

        Some((acceptor, state))
    }

    /// Create a new group from `GroupInfo` bytes
    ///
    /// The bytes are a serialized MLS `MlsMessage` containing `GroupInfo`.
    /// Returns the group ID, acceptor, and state store if successful.
    ///
    /// # Errors
    /// Returns an error if parsing or joining the group fails.
    pub fn create_group(
        &self,
        group_info_bytes: &[u8],
    ) -> Result<(GroupId, GroupAcceptor<C, CS>, GroupStateStore), String> {
        // Parse and observe the group
        let mls_message = MlsMessage::from_bytes(group_info_bytes)
            .map_err(|e| format!("failed to parse MLS message: {e}"))?;

        let external_group = self
            .external_client
            .observe_group(mls_message, None, None)
            .map_err(|e| format!("failed to observe group: {e}"))?;

        // Extract the group ID
        let mls_group_id = external_group.group_context().group_id.clone();
        let group_id = GroupId::from_slice(&mls_group_id);

        // Extract acceptors from the group's extensions
        let acceptors = Self::extract_acceptors_from_group(&external_group);

        // Create the acceptor
        let acceptor = GroupAcceptor::new(
            external_group,
            self.cipher_suite.clone(),
            self.endpoint.secret_key().clone(),
            acceptors,
        );

        // Persist the GroupInfo bytes
        self.state_store
            .store_group(&group_id, group_info_bytes)
            .map_err(|e| format!("failed to persist group: {e}"))?;

        let state = self.state_store.for_group(group_id);

        // Attach state store for epoch roster lookups during validation
        let acceptor = acceptor.with_state_store(state.clone());

        // Store the initial epoch roster for signature validation
        acceptor.store_initial_epoch_roster();

        // Create epoch watcher and learning actor for this group
        self.ensure_epoch_watcher_and_learning(
            group_id,
            acceptor.current_round(),
            &acceptor,
            state.clone(),
        );

        Ok((group_id, acceptor, state))
    }

    /// Store an application message.
    ///
    /// # Errors
    /// Returns an error if the group is not found or storage fails.
    pub fn store_message(
        &self,
        group_id: &GroupId,
        msg: &universal_sync_core::EncryptedAppMessage,
    ) -> Result<u64, String> {
        self.state_store
            .store_app_message(group_id, msg)
            .map_err(|e| format!("failed to store message: {e}"))
    }

    /// Get messages since a sequence number.
    ///
    /// # Errors
    /// Returns an error if the group is not found.
    pub fn get_messages_since(
        &self,
        group_id: &GroupId,
        since_seq: u64,
    ) -> Result<Vec<(u64, universal_sync_core::EncryptedAppMessage)>, String> {
        Ok(self.state_store.get_messages_since(group_id, since_seq))
    }

    /// Subscribe to new messages for a group.
    pub fn subscribe_messages(
        &self,
        group_id: &GroupId,
    ) -> tokio::sync::broadcast::Receiver<universal_sync_core::EncryptedAppMessage> {
        self.state_store.subscribe_messages(group_id)
    }

    /// Get an epoch watcher for a group.
    ///
    /// Returns a watch receiver and a function to get the current epoch.
    /// The acceptor will wait for learning to catch up before processing
    /// proposals for future epochs.
    pub fn get_epoch_watcher(&self, group_id: &GroupId) -> Option<EpochWatcher> {
        let watchers = self.epoch_watchers.read().ok()?;
        let watcher = watchers.get(group_id)?.clone();
        let rx = watcher.rx.clone();
        Some((rx.clone(), Box::new(move || *rx.borrow())))
    }

    /// Notify that an epoch has been learned for a group.
    ///
    /// Call this when a value reaches quorum and is applied.
    pub fn notify_epoch_learned(&self, group_id: &GroupId, epoch: Epoch) {
        if let Ok(watchers) = self.epoch_watchers.read()
            && let Some(watcher) = watchers.get(group_id)
        {
            watcher.notify(epoch);
        }
    }

    /// Ensure an epoch watcher and learning actor exist for a group
    ///
    /// Spawns a learning actor if one doesn't exist for this group.
    fn ensure_epoch_watcher_and_learning(
        &self,
        group_id: GroupId,
        current_epoch: Epoch,
        acceptor: &GroupAcceptor<C, CS>,
        state: GroupStateStore,
    ) {
        let mut watchers = self.epoch_watchers.write().expect("lock poisoned");

        if watchers.contains_key(&group_id) {
            // Already have a learning actor for this group
            return;
        }

        // Create new epoch watcher
        let watcher = Arc::new(GroupEpochWatcher::new(current_epoch));
        watchers.insert(group_id, watcher.clone());

        // Collect initial acceptor addresses
        let initial_acceptors: Vec<_> = acceptor
            .acceptor_addrs()
            .map(|(id, addr)| (*id, addr.clone()))
            .collect();

        // Create and spawn learning actor
        let learning_actor: GroupLearningActor<C, CS> = GroupLearningActor::new(
            self.own_id(),
            group_id,
            self.endpoint.clone(),
            initial_acceptors,
            watcher.tx.clone(),
        );

        tokio::spawn(async move {
            learning_actor.run(state, current_epoch).await;
        });

        tracing::debug!(?group_id, "spawned learning actor");
    }
}
