use crate::LocalDbWorker;
use aead::Aead;
use aes_gcm::aes::cipher::consts::{U12, U16};
use aes_gcm::{Aes128Gcm, Key, KeyInit, Nonce};
use codec::Encode;
use libp2p;
use primitives::data_structure::{ChainSupported, DbTxStateMachine, PeerRecord, UserAccount};
use tokio;

async fn storing_success_n_failed_tx_works() -> Result<(), anyhow::Error> {
    let db_client = LocalDbWorker::initialize_db_client("./dev.db").await?;

    let success_tx = DbTxStateMachine {
        tx_hash: b"0x12345".to_vec(),
        amount: 1000,
        network: ChainSupported::Polkadot,
        success: true,
    };
    let failed_tx = DbTxStateMachine {
        tx_hash: b"0x12222".to_vec(),
        amount: 1320,
        network: ChainSupported::Solana,
        success: false,
    };
    let success_tx_2 = DbTxStateMachine {
        tx_hash: b"0x123454r4".to_vec(),
        amount: 1500,
        network: ChainSupported::Polkadot,
        success: true,
    };
    let failed_tx_2 = DbTxStateMachine {
        tx_hash: b"0x12222ssdx".to_vec(),
        amount: 1600,
        network: ChainSupported::Solana,
        success: false,
    };

    // push to the db
    db_client.update_success_tx(success_tx).await?;
    db_client.update_failed_tx(failed_tx).await?;
    db_client.update_success_tx(success_tx_2).await?;
    db_client.update_failed_tx(failed_tx_2).await?;
    // assert the stored data
    assert_eq!(db_client.get_total_value_success().await?, 2500);
    assert_eq!(db_client.get_total_value_failed().await?, 2920);
    // fetch the streams and assert
    assert_eq!(db_client.get_failed_txs().await?.len(), 2);
    assert_eq!(db_client.get_success_txs().await?.len(), 2);

    Ok(())
}

async fn user_creation_n_retrieving_works() -> Result<(), anyhow::Error> {
    let db_client = LocalDbWorker::initialize_db_client("./dev.db").await?;

    let user_account1 = UserAccount {
        user_name: "Mrisho".to_string(),
        account_id: "5HNLUGWW8nVQa6YrrytRt8TWxTFkd8WerZ5BBapjTv76F79R".to_string(),
        network: ChainSupported::Polkadot,
    };
    let user_account11 = UserAccount {
        user_name: "Mrisho".to_string(),
        account_id: "5CeqYT7dqX9Pup3grzyds1LC6vEMHbNC6hX5BhKw4o97PF9g".to_string(),
        network: ChainSupported::Polkadot,
    };
    let user_account2 = UserAccount {
        user_name: "Mrisho Solana".to_string(),
        account_id: "AhufdbA31tMx1sdgjtqKisNUNHLYs4hvsCwZYQ9YmxTV".to_string(),
        network: ChainSupported::Solana,
    };
    let user_account22 = UserAccount {
        user_name: "Mrisho Ethereum".to_string(),
        account_id: "0x4690152131E5399dE5E76801Fc7742A087829F00".to_string(),
        network: ChainSupported::Ethereum,
    };
    db_client.set_user_account(user_account1).await?;
    db_client.set_user_account(user_account11).await?;
    db_client.set_user_account(user_account2).await?;
    db_client.set_user_account(user_account22).await?;

    assert_eq!(
        db_client
            .get_user_accounts(ChainSupported::Polkadot)
            .await?
            .len(),
        2
    );
    assert_eq!(
        db_client
            .get_user_accounts(ChainSupported::Solana)
            .await?
            .len(),
        1
    );
    assert_eq!(
        db_client
            .get_user_accounts(ChainSupported::Ethereum)
            .await?
            .len(),
        1
    );

    Ok(())
}

async fn storing_user_peer_id_n_retrieving_works() -> Result<(), anyhow::Error> {
    let db_client = LocalDbWorker::initialize_db_client("./dev.db").await?;

    let test_keypair_peer = libp2p::identity::Keypair::generate_ed25519();
    let peer_id = test_keypair_peer.public().to_peer_id().to_base58();
    let bytes_keypair = test_keypair_peer.to_protobuf_encoding().unwrap();

    let key: &[u8] = &[42; 16];
    let key: [u8; 16] = key.try_into()?;
    let key = Key::<Aes128Gcm>::from_slice(&key);
    let cipher = Aes128Gcm::new(&key);
    let nonce = Nonce::from_slice(&key[..12]);
    let encrypted_keypair = cipher.encrypt(nonce, bytes_keypair.as_ref()).unwrap();

    let peer1 = PeerRecord {
        record_id: "recn190".to_string(),
        peer_id: Some(peer_id.clone()),
        account_id1: Some("0x4690152131E5399dE5E76801Fc7742A087829F00".to_string()),
        account_id2: None,
        account_id3: None,
        account_id4: None,
        multi_addr: Some("/ip4/127.0.0.1/tcp/8080".to_string()),
        keypair: Some(encrypted_keypair),
    };
    db_client.record_user_peer_id(peer1.clone()).await?;

    // fetch by account id
    let get_peer1: PeerRecord = db_client
        .get_user_peer_id(
            Some("0x4690152131E5399dE5E76801Fc7742A087829F00".to_string()),
            None,
        )
        .await?
        .into();

    // fetch by peer_id
    let get_peer_1: PeerRecord = db_client
        .get_user_peer_id(None, Some(peer_id))
        .await?
        .into();

    assert_eq!(get_peer1, get_peer_1);
    assert_eq!(get_peer1, peer1);
    Ok(())
}

async fn storing_n_retrieving_saved_peers_works() -> Result<(), anyhow::Error> {
    let db_client = LocalDbWorker::initialize_db_client("./dev.db").await?;

    let test_keypair_peer = libp2p::identity::Keypair::generate_ed25519();
    let peer_id = test_keypair_peer.public().to_peer_id().to_base58();
    let bytes_keypair = test_keypair_peer.to_protobuf_encoding().unwrap();

    let key: &[u8] = &[42; 16];
    let key: [u8; 16] = key.try_into()?;
    let key = Key::<Aes128Gcm>::from_slice(&key);
    let cipher = Aes128Gcm::new(&key);
    let nonce = Nonce::from_slice(&key[..12]);
    let encrypted_keypair = cipher.encrypt(nonce, bytes_keypair.as_ref()).unwrap();

    let saved_peer_1 = PeerRecord {
        record_id: "".to_string(),
        peer_id: Some(peer_id.clone()),
        account_id1: Some("0x4690152131E5399dE5E76801Fc7742A087829F00".to_string()),
        account_id2: None,
        account_id3: None,
        account_id4: None,
        multi_addr: Some("/ip4/127.0.0.1/tcp/8080".to_string()),
        keypair: None,
    };
    db_client
        .record_saved_user_peers(saved_peer_1.clone())
        .await?;

    // retrieving
    let recorded_saved_peer_1: PeerRecord = db_client
        .get_saved_user_peers(saved_peer_1.account_id1.clone().unwrap())
        .await?
        .into();

    assert_eq!(saved_peer_1, recorded_saved_peer_1);
    Ok(())
}

#[tokio::test]
async fn all_db_tests_in_order_works() -> Result<(), anyhow::Error> {
    user_creation_n_retrieving_works().await?;
    storing_user_peer_id_n_retrieving_works().await?;
    storing_success_n_failed_tx_works().await?;
    storing_n_retrieving_saved_peers_works().await?;
    Ok(())
}
