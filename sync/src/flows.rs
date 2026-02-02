//! High-level group creation and joining flows
//!
//! This module provides helper functions for common operations like
//! creating new groups and joining existing ones.

use iroh::Endpoint;
use mls_rs::client_builder::MlsConfig;
use mls_rs::crypto::SignatureSecretKey;
use mls_rs::{CipherSuiteProvider, Client, MlsMessage};

use crate::connector::{ConnectorError, register_group};
use crate::handshake::GroupId;
use crate::learner::GroupLearner;
use crate::proposal::AcceptorId;

/// Error type for flow operations
#[derive(Debug)]
pub enum FlowError {
    /// MLS error
    Mls(mls_rs::error::MlsError),
    /// Connection/registration error
    Connector(ConnectorError),
    /// No acceptors provided
    NoAcceptors,
    /// Failed to generate GroupInfo
    GroupInfo(String),
}

impl std::fmt::Display for FlowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlowError::Mls(e) => write!(f, "MLS error: {e}"),
            FlowError::Connector(e) => write!(f, "connector error: {e}"),
            FlowError::NoAcceptors => write!(f, "no acceptors provided"),
            FlowError::GroupInfo(e) => write!(f, "failed to generate GroupInfo: {e}"),
        }
    }
}

impl std::error::Error for FlowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FlowError::Mls(e) => Some(e),
            FlowError::Connector(e) => Some(e),
            _ => None,
        }
    }
}

impl From<mls_rs::error::MlsError> for FlowError {
    fn from(e: mls_rs::error::MlsError) -> Self {
        FlowError::Mls(e)
    }
}

impl From<ConnectorError> for FlowError {
    fn from(e: ConnectorError) -> Self {
        FlowError::Connector(e)
    }
}

/// Result of creating a new group
pub struct CreatedGroup<C, CS>
where
    C: MlsConfig + Clone,
    CS: CipherSuiteProvider,
{
    /// The group learner for this device
    pub learner: GroupLearner<C, CS>,
    /// The group ID
    pub group_id: GroupId,
    /// The GroupInfo message bytes (for sharing with others)
    pub group_info: Vec<u8>,
}

/// Create a new MLS group and register it with acceptors
///
/// This is the main entry point for creating a new synchronized group.
/// It performs the following steps:
/// 1. Creates a new MLS group using the provided client
/// 2. Generates a GroupInfo message
/// 3. Registers the group with all provided acceptors
///
/// # Arguments
/// * `client` - The MLS client to use for creating the group
/// * `signer` - The signing key for this device
/// * `cipher_suite` - The cipher suite provider
/// * `endpoint` - The iroh endpoint for connecting to acceptors
/// * `acceptors` - The set of acceptors to register with
///
/// # Returns
/// A [`CreatedGroup`] containing the learner, group ID, and GroupInfo.
///
/// # Example
///
/// ```ignore
/// let created = create_group(
///     &client,
///     signer,
///     cipher_suite,
///     &endpoint,
///     &acceptor_ids,
/// ).await?;
///
/// // Share created.group_info with other devices so they can join
/// let connector = IrohConnector::new(endpoint, created.group_id);
/// ```
pub async fn create_group<C, CS>(
    client: &Client<C>,
    signer: SignatureSecretKey,
    cipher_suite: CS,
    endpoint: &Endpoint,
    acceptors: &[AcceptorId],
) -> Result<CreatedGroup<C, CS>, FlowError>
where
    C: MlsConfig + Clone,
    CS: CipherSuiteProvider + Clone,
{
    if acceptors.is_empty() {
        return Err(FlowError::NoAcceptors);
    }

    // Create a new MLS group
    let group = client.create_group(Default::default(), Default::default())?;

    // Generate GroupInfo for external observers (acceptors)
    let group_info_msg = group.group_info_message(true)?;
    let group_info = group_info_msg.to_bytes()?;

    // Extract the group ID
    let mls_group_id = group.context().group_id.clone();
    let group_id = GroupId::from_slice(&mls_group_id);

    // Register with all acceptors
    for acceptor_id in acceptors {
        register_group(endpoint, acceptor_id, &group_info).await?;
    }

    // Create the learner
    let learner = GroupLearner::new(group, signer, cipher_suite, acceptors.iter().copied());

    Ok(CreatedGroup {
        learner,
        group_id,
        group_info,
    })
}

/// Result of joining an existing group
pub struct JoinedGroup<C, CS>
where
    C: MlsConfig + Clone,
    CS: CipherSuiteProvider,
{
    /// The group learner for this device
    pub learner: GroupLearner<C, CS>,
    /// The group ID
    pub group_id: GroupId,
}

/// Join an existing MLS group using a Welcome message
///
/// This is the main entry point for joining an existing synchronized group.
/// A Welcome message is received from another group member who has added
/// this device to the group.
///
/// # Arguments
/// * `client` - The MLS client to use
/// * `signer` - The signing key for this device
/// * `cipher_suite` - The cipher suite provider
/// * `welcome_bytes` - The serialized Welcome message
/// * `acceptors` - The set of acceptors for this group
///
/// # Returns
/// A [`JoinedGroup`] containing the learner and group ID.
///
/// # Example
///
/// ```ignore
/// // Receive welcome_bytes from another group member
/// let joined = join_group(
///     &client,
///     signer,
///     cipher_suite,
///     &welcome_bytes,
///     &acceptor_ids,
/// ).await?;
///
/// let connector = IrohConnector::new(endpoint, joined.group_id);
/// ```
pub async fn join_group<C, CS>(
    client: &Client<C>,
    signer: SignatureSecretKey,
    cipher_suite: CS,
    welcome_bytes: &[u8],
    acceptors: &[AcceptorId],
) -> Result<JoinedGroup<C, CS>, FlowError>
where
    C: MlsConfig + Clone,
    CS: CipherSuiteProvider + Clone,
{
    // Parse the Welcome message
    let welcome = MlsMessage::from_bytes(welcome_bytes)?;

    // Join the group
    let (group, _info) = client.join_group(None, &welcome)?;

    // Extract the group ID
    let mls_group_id = group.context().group_id.clone();
    let group_id = GroupId::from_slice(&mls_group_id);

    // Create the learner
    let learner = GroupLearner::new(group, signer, cipher_suite, acceptors.iter().copied());

    Ok(JoinedGroup { learner, group_id })
}
