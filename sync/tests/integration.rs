//! Integration tests for the sync crate
//!
//! These tests verify the networking layer and MLS integration.

use std::time::Duration;

use bytes::Bytes;
use iroh::Endpoint;
use mls_rs::external_client::ExternalClient;
use tempfile::TempDir;
use tracing_subscriber::{EnvFilter, fmt};
use universal_sync::testing::{
    test_cipher_suite, test_client, test_crypto_provider, test_identity_provider,
};
use universal_sync::{
    AcceptorRegistry, GroupId, PAXOS_ALPN, SharedFjallStateStore, accept_connection,
    register_group_with_addr,
};

/// Initialize tracing for tests
fn init_tracing() {
    let _ = fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("universal_sync=debug")),
        )
        .with_test_writer()
        .try_init();
}

/// Create an iroh endpoint for testing
async fn test_endpoint() -> Endpoint {
    Endpoint::builder()
        .alpns(vec![PAXOS_ALPN.to_vec()])
        .bind()
        .await
        .expect("failed to create endpoint")
}

#[tokio::test]
async fn test_state_store_group_persistence() {
    init_tracing();

    let temp_dir = TempDir::new().unwrap();

    // Create and store some groups
    let group_id_1 = GroupId::new([1u8; 32]);
    let group_id_2 = GroupId::new([2u8; 32]);
    let group_info_1 = b"test group info 1".to_vec();
    let group_info_2 = b"test group info 2".to_vec();

    {
        let store = SharedFjallStateStore::open(temp_dir.path())
            .await
            .expect("open store");

        store
            .store_group(&group_id_1, &group_info_1)
            .expect("store 1");
        store
            .store_group(&group_id_2, &group_info_2)
            .expect("store 2");

        // Verify they're stored
        let groups = store.list_groups();
        assert_eq!(groups.len(), 2);
    }

    // Reopen and verify persistence
    {
        let store = SharedFjallStateStore::open(temp_dir.path())
            .await
            .expect("reopen store");

        let groups = store.list_groups();
        assert_eq!(groups.len(), 2);

        let info_1 = store.get_group_info(&group_id_1);
        assert_eq!(info_1, Some(group_info_1));

        let info_2 = store.get_group_info(&group_id_2);
        assert_eq!(info_2, Some(group_info_2));
    }
}

#[tokio::test]
async fn test_iroh_connection_roundtrip() {
    init_tracing();

    // Create two endpoints
    let endpoint_a = test_endpoint().await;
    let endpoint_b = test_endpoint().await;

    let addr_b = endpoint_b.addr();

    // Spawn receiver
    let recv_task = tokio::spawn({
        let endpoint_b = endpoint_b.clone();
        async move {
            use futures::{SinkExt, StreamExt};
            use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

            let incoming = endpoint_b.accept().await.unwrap();
            let conn = incoming.accept().unwrap().await.unwrap();
            let (send, recv) = conn.accept_bi().await.unwrap();

            let codec = LengthDelimitedCodec::new();
            let mut reader = FramedRead::new(recv, codec.clone());
            let mut writer = FramedWrite::new(send, codec);

            // Echo received messages
            while let Some(Ok(msg)) = reader.next().await {
                writer.send(msg.freeze()).await.unwrap();
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    use futures::{SinkExt, StreamExt};
    use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

    let conn = endpoint_a.connect(addr_b, PAXOS_ALPN).await.unwrap();
    let (send, recv) = conn.open_bi().await.unwrap();

    let codec = LengthDelimitedCodec::new();
    let mut reader = FramedRead::new(recv, codec.clone());
    let mut writer = FramedWrite::new(send, codec);

    // Send and receive
    let test_msg = Bytes::from("hello world");
    writer.send(test_msg.clone()).await.unwrap();

    let echoed = reader.next().await.unwrap().unwrap();
    assert_eq!(echoed.as_ref(), test_msg.as_ref());

    drop(writer);
    recv_task.abort();
}

#[tokio::test]
async fn test_mls_group_registration() {
    init_tracing();

    // Setup directories
    let acceptor_dir = TempDir::new().unwrap();

    // Create acceptor endpoint and registry
    let acceptor_endpoint = test_endpoint().await;
    let acceptor_addr = acceptor_endpoint.addr();

    let crypto = test_crypto_provider();
    let cipher_suite = test_cipher_suite(&crypto);

    let external_client = ExternalClient::builder()
        .crypto_provider(crypto.clone())
        .identity_provider(test_identity_provider())
        .build();

    let state_store = SharedFjallStateStore::open(acceptor_dir.path())
        .await
        .expect("open state store");

    let registry =
        AcceptorRegistry::new(external_client, cipher_suite.clone(), state_store.clone());

    // Spawn acceptor server
    let acceptor_task = tokio::spawn({
        let acceptor_endpoint = acceptor_endpoint.clone();
        let registry = registry.clone();
        async move {
            if let Some(incoming) = acceptor_endpoint.accept().await {
                let _ = accept_connection(incoming, registry).await;
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Create client
    let client_endpoint = test_endpoint().await;
    let test_result = test_client("alice");

    // Create MLS group
    let group = test_result
        .client
        .create_group_with_id(
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            Default::default(),
            Default::default(),
        )
        .expect("create group");

    // Generate GroupInfo
    let group_info_msg = group.group_info_message(true).expect("group info");
    let group_info_bytes = group_info_msg.to_bytes().expect("serialize");

    // Register with acceptor
    let result = register_group_with_addr(&client_endpoint, acceptor_addr, &group_info_bytes).await;
    assert!(result.is_ok(), "registration failed: {:?}", result);

    // Verify group was persisted
    let mls_group_id = group.context().group_id.clone();
    let group_id = GroupId::from_slice(&mls_group_id);

    // Give acceptor time to persist
    tokio::time::sleep(Duration::from_millis(100)).await;

    let stored = state_store.get_group_info(&group_id);
    assert!(stored.is_some(), "group should be persisted");

    acceptor_task.abort();
}
