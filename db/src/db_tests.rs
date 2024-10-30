use crate::DbWorker;
use aead::Aead;
use aes_gcm::aes::cipher::consts::{U12, U16};
use aes_gcm::{Aes128Gcm, Key, KeyInit, Nonce};
use codec::Encode;
use libp2p;
use primitives::data_structure::{ChainSupported, DbTxStateMachine, PeerRecord, UserAccount};
use tokio;

async fn storing_success_n_failed_tx_works() -> Result<(), anyhow::Error> {
    let db_client = DbWorker::initialize_db_client("./dev.db").await?;

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
    let db_client = DbWorker::initialize_db_client("./dev.db").await?;

    let user_account1 = UserAccount {
        user_name: "Mrisho".encode(),
        account_id: "5HNLUGWW8nVQa6YrrytRt8TWxTFkd8WerZ5BBapjTv76F79R".encode(),
        network: ChainSupported::Polkadot,
    };
    let user_account11 = UserAccount {
        user_name: "Mrisho".encode(),
        account_id: "5CeqYT7dqX9Pup3grzyds1LC6vEMHbNC6hX5BhKw4o97PF9g".encode(),
        network: ChainSupported::Polkadot,
    };
    let user_account2 = UserAccount {
        user_name: "Mrisho Solana".encode(),
        account_id: "AhufdbA31tMx1sdgjtqKisNUNHLYs4hvsCwZYQ9YmxTV".encode(),
        network: ChainSupported::Solana,
    };
    let user_account22 = UserAccount {
        user_name: "Mrisho Ethereum".encode(),
        account_id: "0x4690152131E5399dE5E76801Fc7742A087829F00".encode(),
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
    let db_client = DbWorker::initialize_db_client("./dev.db").await?;

    let test_keypair_peer = libp2p::identity::Keypair::generate_ed25519();
    let peer_address = test_keypair_peer.public().to_peer_id().to_base58();
    let bytes_keypair = test_keypair_peer.to_protobuf_encoding().unwrap();

    let key: &[u8] = &[42; 16];
    let key: [u8; 16] = key.try_into()?;
    let key = Key::<Aes128Gcm>::from_slice(&key);
    let cipher = Aes128Gcm::new(&key);
    let nonce = Nonce::from_slice(&key[..12]);
    let encrypted_keypair = cipher.encrypt(nonce, bytes_keypair.as_ref()).unwrap();

    let peer1 = PeerRecord {
        peer_address,
        account_id1: Some("0x4690152131E5399dE5E76801Fc7742A087829F00".encode()),
        account_id2: None,
        account_id3: None,
        account_id4: None,
        multi_addr: "/ip4/127.0.0.1/tcp/8080".to_string(),
        keypair: Some(encrypted_keypair),
    };
    db_client.record_user_peer_id(peer1.clone()).await?;

    let get_peer1: PeerRecord = db_client
        .get_user_peer_id("0x4690152131E5399dE5E76801Fc7742A087829F00".encode())
        .await?
        .into();
    assert_eq!(get_peer1, peer1);
    Ok(())
}

#[tokio::test]
async fn all_db_tests_in_order_works() -> Result<(), anyhow::Error> {
    user_creation_n_retrieving_works().await?;
    storing_user_peer_id_n_retrieving_works().await?;
    storing_success_n_failed_tx_works().await?;
    Ok(())
}
