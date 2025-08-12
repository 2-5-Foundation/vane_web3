#![allow(unused_imports)]
#![allow(unused)]
extern crate alloc;

pub mod db;

#[cfg(test)]
mod db_tests;
use crate::db::read_filters::{BoolFilter, StringFilter};
use crate::db::saved_peers::Data;
use crate::db::transactions_data::{UniqueWhereParam, WhereParam};
use crate::db::{
    PrismaClient, PrismaClientBuilder, UserPeerScalarFieldEnum, new_client_with_url, nonce, port,
    read_filters::{BigIntFilter, BytesFilter, IntFilter},
    saved_peer_account_info, saved_peers, transaction, transactions_data, user_account, user_peer,
    user_peer_account_info,
};
use alloc::sync::Arc;
use anyhow::{Error, anyhow};
use codec::{Decode, Encode};
use hex;
use log::{debug, error, info, trace, warn};
use primitives::data_structure::{
    AccountInfo, ChainSupported, DbTxStateMachine, PeerRecord, UserAccount,
};
use prisma_client_rust::{BatchItem, Direction, PrismaValue, Raw, query_core::RawQuery};

use primitives::data_structure::{DbWorkerInterface, Ports};
use serde::{Deserialize, Serialize};
use std::future::Future;

/// Handling connection and interaction with the local database
#[derive(Clone)]
pub struct LocalDbWorker {
    db: Arc<PrismaClient>,
}

const SERVER_DATA_ID: i32 = 1;

impl DbWorkerInterface for LocalDbWorker {
    async fn initialize_db_client(file_url: &str) -> Result<Self, anyhow::Error> {
        let url = format!("file:{}", file_url);
        let client = new_client_with_url(&url)
            .await
            .map_err(|err| anyhow!("failed to initialize db client, caused by: {err}"))?;

        let client = Arc::new(client);

        cfg!(feature = "e2e");
        client._migrate_deploy().await?;

        // we are initializing transaction data as all of following operations is going to be updating this storage item
        let return_data = client
            .transactions_data()
            .find_first(vec![WhereParam::Id(IntFilter::Equals(1))])
            .exec()
            .await;

        if let Ok(return_data) = return_data {
            if let None = return_data {
                client
                    .transactions_data()
                    .create(0, 0, vec![])
                    .exec()
                    .await?;
            }
        } else {
            // create new tx data
            if let Err(err) = client.transactions_data().create(0, 0, vec![]).exec().await {
                error!(target:"db","failed to create new transaction data; caused by: {err}");
            }
        }
        Ok(Self { db: client })
    }

    async fn set_user_account(&self, user: UserAccount) -> Result<(), anyhow::Error> {
        self.db
            .user_account()
            .create(
                user.user_name,
                user.account_id,
                user.network.into(),
                Default::default(),
            )
            .exec()
            .await?;
        Ok(())
    }

    async fn increment_nonce(&self) -> Result<(), anyhow::Error> {
        self.db
            .nonce()
            .update(nonce::id::equals(1), vec![nonce::nonce::increment(1)])
            .exec()
            .await?;
        Ok(())
    }

    async fn get_nonce(&self) -> Result<u32, anyhow::Error> {
        let mut nonce = 0;
        let nonce_data = self
            .db
            .nonce()
            .find_unique(nonce::UniqueWhereParam::IdEquals(1))
            .exec()
            .await?;
        if nonce_data.is_none() {
            // create the entity
            self.db.nonce().create(0, vec![]).exec().await?;
        } else {
            nonce = nonce_data.unwrap().nonce
        }
        Ok(nonce as u32)
    }

    // get all related network id accounts
    async fn get_user_accounts(
        &self,
        network: ChainSupported,
    ) -> Result<Vec<UserAccount>, anyhow::Error> {
        let accounts = self
            .db
            .user_account()
            .find_many(vec![user_account::WhereParam::NetworkId(
                StringFilter::Equals(network.into()),
            )])
            .exec()
            .await?;
        let accounts = accounts
            .into_iter()
            .map(|acc| acc.into())
            .collect::<Vec<UserAccount>>();
        Ok(accounts)
    }

    async fn update_success_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error> {
        let tx = self
            .db
            .transaction()
            .create(
                tx_state.tx_hash,
                tx_state.amount as i64,
                tx_state.network.into(),
                tx_state.success,
                Default::default(),
            )
            .exec()
            .await?;

        self.db
            .transactions_data()
            .update(
                transactions_data::id::equals(1),
                vec![transactions_data::success_value::increment(
                    tx_state.amount as i64,
                )],
            )
            .exec()
            .await?;

        Ok(())
    }

    async fn update_failed_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error> {
        let tx = self
            .db
            .transaction()
            .create(
                tx_state.tx_hash,
                tx_state.amount as i64,
                tx_state.network.into(),
                tx_state.success,
                Default::default(),
            )
            .exec()
            .await?;

        self.db
            .transactions_data()
            .update(
                transactions_data::id::equals(1),
                vec![transactions_data::failed_value::increment(
                    tx_state.amount as i64,
                )],
            )
            .exec()
            .await?;
        info!(target: "db","updated failed transaction in local db");
        Ok(())
    }

    async fn get_failed_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error> {
        let failed_txs = self
            .db
            .transaction()
            .find_many(vec![transaction::WhereParam::Status(BoolFilter::Equals(
                false,
            ))])
            .exec()
            .await?;
        let failed_txs = failed_txs
            .into_iter()
            .map(|txn| txn.into())
            .collect::<Vec<DbTxStateMachine>>();
        Ok(failed_txs)
    }

    async fn get_success_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error> {
        let success_txs = self
            .db
            .transaction()
            .find_many(vec![transaction::WhereParam::Status(BoolFilter::Equals(
                true,
            ))])
            .exec()
            .await?;
        let success_txs = success_txs
            .into_iter()
            .map(|txn| txn.into())
            .collect::<Vec<DbTxStateMachine>>();
        Ok(success_txs)
    }

    async fn get_total_value_success(&self) -> Result<u64, anyhow::Error> {
        let main_data = self
            .db
            .transactions_data()
            .find_unique(transactions_data::id::equals(SERVER_DATA_ID))
            .exec()
            .await?
            .ok_or(anyhow!(
                "Main Data not found, shouldnt happen must initailize"
            ))?;
        let success_value = main_data.success_value as u64;
        Ok(success_value)
    }

    async fn get_total_value_failed(&self) -> Result<u64, anyhow::Error> {
        let main_data = self
            .db
            .transactions_data()
            .find_unique(transactions_data::id::equals(SERVER_DATA_ID))
            .exec()
            .await?
            .ok_or(anyhow!(
                "Main Data not found, shouldnt happen must initailize"
            ))?;
        let failed_value = main_data.failed_value as u64;
        Ok(failed_value)
    }

    async fn record_user_peer_id(&self, peer_record: PeerRecord) -> Result<(), anyhow::Error> {
        let client = &self.db;

        client
            ._transaction()
            .run(|tx| async move {
                let user_peer = tx
                    .user_peer()
                    .create(
                        "MAIN_USER_ID".to_string(),
                        peer_record.peer_id.unwrap_or_default(),
                        peer_record.multi_addr.unwrap_or_default(),
                        peer_record.keypair.unwrap_or_default(),
                        vec![],
                    )
                    .exec()
                    .await?;

                for account_info in peer_record.accounts {
                    tx.user_peer_account_info()
                        .create(
                            account_info.account,
                            account_info.network.to_string(),
                            db::user_peer::UniqueWhereParam::IdEquals("MAIN_USER_ID".to_string()),
                            vec![],
                        )
                        .exec()
                        .await?;
                }

                Ok(())
            })
            .await
    }

    async fn update_user_peer_id_account_ids(
        &self,
        account: AccountInfo,
    ) -> Result<(), anyhow::Error> {
        println!("updating peer record: {:?}", &account);
        let client = &self.db;
        client
            .user_peer_account_info()
            .create(
                account.account,
                account.network.to_string(),
                db::user_peer::UniqueWhereParam::IdEquals("MAIN_USER_ID".to_string()),
                vec![],
            )
            .exec()
            .await?;

        Ok(())
    }

    // get peer by account id by either account id or peerId
    async fn get_user_peer_id(
        &self,
        account_id: Option<String>,
        peer_id: Option<String>,
    ) -> Result<PeerRecord, anyhow::Error> {
        let where_param = match (account_id, peer_id) {
            (Some(acc_id), _) => user_peer::WhereParam::AccountsEvery(vec![
                user_peer_account_info::WhereParam::AccountId(StringFilter::Equals(acc_id)),
            ]),
            (_, Some(pid)) => user_peer::WhereParam::PeerId(StringFilter::Equals(pid)),
            (None, None) => return Err(anyhow!("Please provide either account ID or peer ID")),
        };

        let peer = self
            .db
            .user_peer()
            .find_first(vec![where_param])
            .exec()
            .await?
            .ok_or_else(|| anyhow!("Peer not found in DB"))?;
        println!("data: {peer:?}");
        Ok(peer.into())
    }

    // set port ids {
    async fn set_ports(&self, rpc: u16, p2p: u16) -> Result<(), anyhow::Error> {
        self.db
            .port()
            .create(rpc as i64, p2p as i64, Default::default())
            .exec()
            .await?;

        Ok(())
    }

    // get port ids
    async fn get_ports(&self) -> Result<Option<Ports>, anyhow::Error> {
        let ports = self
            .db
            .port()
            .find_unique(port::UniqueWhereParam::IdEquals(1))
            .exec()
            .await?;
        let ports = if let Some(port) = ports {
            Some(port.into())
        } else {
            None
        };
        Ok(ports)
    }

    // saved peers interacted with
    async fn record_saved_user_peers(&self, peer_record: PeerRecord) -> Result<(), anyhow::Error> {
        let client = &self.db;

        client
            ._transaction()
            .run(|tx| async move {
                // Create the SavedPeer first
                let saved_peer = tx
                    .saved_peers()
                    .create(
                        peer_record.peer_id.unwrap_or_default(),
                        peer_record.multi_addr.unwrap_or_default(),
                        vec![],
                    )
                    .exec()
                    .await?;

                // Then create all associated account info records
                for account_info in peer_record.accounts {
                    tx.saved_peer_account_info() // Assuming this is your related model
                        .create(
                            account_info.account,
                            account_info.network.to_string(),
                            db::saved_peers::UniqueWhereParam::IdEquals(saved_peer.id.clone()),
                            vec![],
                        )
                        .exec()
                        .await?;
                }

                Ok(())
            })
            .await
    }

    // get saved peers
    async fn get_saved_user_peers(&self, account_id: String) -> Result<String, anyhow::Error> {
        let peer_data = self
            .db
            .saved_peers()
            .find_first(vec![saved_peers::WhereParam::AccountsEvery(vec![
                saved_peer_account_info::WhereParam::AccountId(StringFilter::Equals(account_id)),
            ])])
            .exec()
            .await?
            .ok_or(anyhow!("Peer Not found in DB"))?;

        Ok(peer_data.node_id)
    }

    // get all saved peers
    async fn get_all_saved_peers(&self) -> Result<(Vec<String>, String), anyhow::Error> {
        let peers = self.db.saved_peers().find_many(vec![]).exec().await?;

        if peers.is_empty() {
            return Err(anyhow!("No saved peers found"));
        }

        // Verify all peers have the same node_id (peer_id)
        let first_peer_id = &peers[0].node_id;
        for peer in &peers {
            if peer.node_id != *first_peer_id {
                return Err(anyhow!(
                    "Inconsistent peer ID mapping: expected {}, got {}",
                    first_peer_id,
                    peer.node_id
                ));
            }
        }

        // Collect all account IDs from all peers
        let mut all_account_ids = Vec::new();
        for peer_data in peers {
            let account_infos = self
                .db
                .saved_peer_account_info()
                .find_many(vec![saved_peer_account_info::WhereParam::SavedPeerId(
                    StringFilter::Equals(peer_data.id),
                )])
                .exec()
                .await?;

            for acc in account_infos {
                all_account_ids.push(acc.account_id);
            }
        }

        Ok((all_account_ids, first_peer_id.clone()))
    }

    // delete a specific saved peer
    async fn delete_saved_peer(&self, peer_id: &str) -> Result<(), anyhow::Error> {
        // First find the peer to get its ID
        let peer = self
            .db
            .saved_peers()
            .find_first(vec![saved_peers::WhereParam::NodeId(StringFilter::Equals(
                peer_id.to_string(),
            ))])
            .exec()
            .await?
            .ok_or_else(|| anyhow!("Peer not found with peer_id: {}", peer_id))?;

        // Delete the peer (this should cascade to related account info records)
        self.db
            .saved_peers()
            .delete(saved_peers::UniqueWhereParam::IdEquals(peer.id))
            .exec()
            .await?;

        Ok(())
    }

    // get a saved peer directly by peer_id
    async fn get_saved_peer_by_id(&self, peer_id: &str) -> Result<Option<String>, anyhow::Error> {
        let peer = self
            .db
            .saved_peers()
            .find_first(vec![saved_peers::WhereParam::NodeId(StringFilter::Equals(
                peer_id.to_string(),
            ))])
            .exec()
            .await?;

        Ok(peer.map(|p| p.node_id))
    }

    // update an existing saved peer
    async fn update_saved_peer(&self, peer_record: PeerRecord) -> Result<(), anyhow::Error> {
        let client = &self.db;

        client
            ._transaction()
            .run(|tx| async move {
                // First, find the existing peer
                let peer_id = peer_record
                    .peer_id
                    .ok_or_else(|| anyhow!("PeerRecord must have a peer_id"))?;

                let existing_peer = tx
                    .saved_peers()
                    .find_unique(saved_peers::UniqueWhereParam::NodeIdEquals(peer_id.clone()))
                    .exec()
                    .await?
                    .ok_or_else(|| anyhow!("Peer not found for update"))?;

                // Update the peer information
                tx.saved_peers()
                    .update(
                        saved_peers::UniqueWhereParam::IdEquals(existing_peer.id),
                        vec![saved_peers::multi_addr::set(
                            peer_record.multi_addr.unwrap_or_default(),
                        )],
                    )
                    .exec()
                    .await?;

                // Delete existing account info records
                tx.saved_peer_account_info()
                    .delete_many(vec![saved_peer_account_info::WhereParam::SavedPeerId(
                        StringFilter::Equals(existing_peer.id),
                    )])
                    .exec()
                    .await?;

                // Create new account info records
                for account_info in peer_record.accounts {
                    tx.saved_peer_account_info()
                        .create(
                            account_info.account,
                            account_info.network.to_string(),
                            db::saved_peers::UniqueWhereParam::IdEquals(existing_peer.id.clone()),
                            vec![],
                        )
                        .exec()
                        .await?;
                }

                Ok(())
            })
            .await
    }

    // get all account IDs that are mapped to a specific peer ID
    async fn get_account_ids_by_peer_id(
        &self,
        peer_id: &str,
    ) -> Result<Vec<String>, anyhow::Error> {
        let peer = self
            .db
            .saved_peers()
            .find_first(vec![saved_peers::WhereParam::NodeId(StringFilter::Equals(
                peer_id.to_string(),
            ))])
            .exec()
            .await?
            .ok_or_else(|| anyhow!("Peer not found with peer_id: {}", peer_id))?;

        let account_infos = self
            .db
            .saved_peer_account_info()
            .find_many(vec![saved_peer_account_info::WhereParam::SavedPeerId(
                StringFilter::Equals(peer.id),
            )])
            .exec()
            .await?;

        let account_ids: Vec<String> = account_infos
            .into_iter()
            .map(|acc| acc.account_id)
            .collect();

        Ok(account_ids)
    }
}

// Type convertions
#[cfg(not(target_arch = "wasm32"))]
impl From<user_peer::Data> for PeerRecord {
    fn from(data: user_peer::Data) -> Self {
        let mut accounts = vec![];
        if let Some(accs) = data.accounts {
            accs.into_iter().for_each(|acc| {
                accounts.push(acc.into());
            });
        }
        PeerRecord {
            peer_id: Some(data.peer_id),
            accounts,
            multi_addr: Some(data.multi_addr),
            keypair: Some(data.keypair.to_vec()),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<user_peer_account_info::Data> for AccountInfo {
    fn from(data: user_peer_account_info::Data) -> Self {
        AccountInfo {
            account: data.account_id,
            network: ChainSupported::from(data.network.as_str()),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<saved_peer_account_info::Data> for AccountInfo {
    fn from(data: saved_peer_account_info::Data) -> Self {
        AccountInfo {
            account: data.account_id,
            network: ChainSupported::from(data.network.as_str()),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<saved_peers::Data> for PeerRecord {
    fn from(data: saved_peers::Data) -> Self {
        let mut accounts = vec![];
        if let Some(accs) = data.accounts {
            accs.into_iter().for_each(|acc| {
                accounts.push(acc.into());
            });
        }
        PeerRecord {
            peer_id: Some(data.node_id),
            accounts,
            multi_addr: Some(data.multi_addr),
            keypair: None, // SavedPeers doesn't store keypair
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<user_account::Data> for UserAccount {
    fn from(value: user_account::Data) -> Self {
        Self {
            user_name: value.username,
            account_id: value.account_id,
            network: ChainSupported::from(value.network_id.as_str()),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<transaction::Data> for DbTxStateMachine {
    fn from(value: transaction::Data) -> Self {
        Self {
            tx_hash: value.tx_hash,
            amount: value
                .value
                .try_into()
                .expect("failed to convert u128 to u64"),
            network: ChainSupported::from(value.network.as_str()),
            success: value.status,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<port::Data> for Ports {
    fn from(value: db::port::Data) -> Self {
        Self {
            rpc: value.rpc_port as u16,
            p_2_p_port: value.p_2_p_port as u16,
        }
    }
}
