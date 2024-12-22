#![allow(unused_imports)]
#![allow(unused)]
extern crate alloc;

#[cfg(not(target_arch = "wasm32"))]
pub mod db;

#[cfg(test)]
mod db_tests;
#[cfg(not(target_arch = "wasm32"))]
use crate::db::read_filters::{BoolFilter, StringFilter};
#[cfg(not(target_arch = "wasm32"))]
use crate::db::transactions_data::{UniqueWhereParam, WhereParam};
#[cfg(not(target_arch = "wasm32"))]
use crate::db::{
    new_client_with_url, nonce, port,
    read_filters::{BigIntFilter, BytesFilter, IntFilter},
    saved_peers, transaction, transactions_data, user_account, user_peer, PrismaClient,
    PrismaClientBuilder, UserPeerScalarFieldEnum,
};
use alloc::sync::Arc;
use anyhow::{anyhow, Error};
use codec::{Decode, Encode};
use hex;
use log::{debug, error, info, trace, warn};
use primitives::data_structure::{ChainSupported, DbTxStateMachine, PeerRecord, UserAccount};
#[cfg(not(target_arch = "wasm32"))]
use prisma_client_rust::{query_core::RawQuery, BatchItem, Direction, PrismaValue, Raw};
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::future::Future;
#[cfg(target_arch = "wasm32")]
use redb::{Database, ReadableTable, TableDefinition};
#[cfg(target_arch = "wasm32")]
use web_sys::{FileSystemDirectoryHandle, StorageManager};
#[cfg(not(target_arch = "wasm32"))]
use crate::db::saved_peers::Data;


// ======================================= Define table schemas =============================== //
#[cfg(target_arch = "wasm32")]
const USER_ACCOUNT_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("user_accounts");
#[cfg(target_arch = "wasm32")]
const PORT_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("ports");
#[cfg(target_arch = "wasm32")]
const TRANSACTIONS_DATA_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("transactions_data");
// stores array of tx but all are encoded
#[cfg(target_arch = "wasm32")]
const TRANSACTION_TABLE: TableDefinition<&str, Vec<Vec<u8>>> = TableDefinition::new("transactions");
#[cfg(target_arch = "wasm32")]
const NONCE_TABLE: TableDefinition<&str, u32> = TableDefinition::new("nonce");
// stores array of user profiles
#[cfg(target_arch = "wasm32")]
const USER_PEER_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("user_peers");

// stores array of saved peers
#[cfg(target_arch = "wasm32")]
const SAVED_PEERS_TABLE: TableDefinition<&str, Vec<Vec<u8>>> = TableDefinition::new("saved_peers");

// ===================================== DB KEYS ====================================== //
#[cfg(target_arch = "wasm32")]
pub const USER_ACC_KEY:&str = "user_account";
#[cfg(target_arch = "wasm32")]
pub const NONCE_KEY:&str = "nonce_key";
#[cfg(target_arch = "wasm32")]
pub const TXS_KEY:&str = "txs_key";
#[cfg(target_arch = "wasm32")]
pub const TXS_DATA_KEY:&str = "txs_data_key";
#[cfg(target_arch = "wasm32")]
pub const USER_PEER_RECORD_KEY:&str = "user_peer";
#[cfg(target_arch = "wasm32")]
pub const SAVED_PEERS_KEY: &str = "saved_peers";
#[cfg(target_arch = "wasm32")]
pub const PORTS_KEY:&str = "saved_ports";

pub enum DbEngine {
    NativeLocal,
    BrowserWasm,
    Mobile
}

/// db interface
#[allow(async_fn_in_trait)]
pub trait DbWorkerInterface:Sized {
    async fn initialize_db_client(file_url: &str) -> Result<Self, anyhow::Error>;

    async fn set_user_account(&self, user: UserAccount) -> Result<(), anyhow::Error>;

    async fn get_nonce(&self) -> Result<u32, anyhow::Error>;

    // get all related network id accounts
    async fn get_user_accounts(
        &self,
        network: ChainSupported,
    ) -> Result<Vec<UserAccount>, anyhow::Error>;

    async fn update_success_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error>;

    async fn update_failed_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error>;
    async fn get_failed_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error>;

    async fn get_total_value_success(&self) -> Result<u64, anyhow::Error>;
    async fn get_total_value_failed(&self) -> Result<u64, anyhow::Error>;

    async fn record_user_peer_id(&self, peer_record: PeerRecord) -> Result<(), anyhow::Error>;

    async fn update_user_peer_id_accounts(
        &self,
        peer_record: PeerRecord,
    ) -> Result<(), anyhow::Error>;

    async fn get_success_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error>;

    // get peer by account id by either account id or peerId
    async fn get_user_peer_id(
        &self,
        account_id: Option<String>,
        peer_id: Option<String>,
    ) -> Result<PeerRecord, anyhow::Error>;

    async fn increment_nonce(&self) -> Result<(), anyhow::Error>;
    // set port ids {
    async fn set_ports(&self, rpc: u16, p2p: u16) -> Result<(), anyhow::Error>;
    // get port ids
    async fn get_ports(&self) -> Result<Option<Ports>, anyhow::Error>;

    // saved peers interacted with
    async fn record_saved_user_peers(
        &self,
        peer_record: PeerRecord,
    ) -> Result<(), anyhow::Error>;

    // get saved peers
    async fn get_saved_user_peers(
        &self,
        account_id: String,
    ) -> Result<PeerRecord, anyhow::Error>;
}

/// handling connection and interaction with the browser based OPFS database
#[cfg(target_arch = "wasm32")]
pub struct OpfsRedbWorker {
    db: Database,
}

// ============================== REDB SCHEMA ================================ //
#[cfg(target_arch = "wasm32")]
#[derive(Serialize, Deserialize, Encode, Decode)]
struct TransactionsData {
    success_value: i64,
    failed_value: i64,
}

#[cfg(target_arch = "wasm32")]
#[derive(Serialize, Deserialize, Encode, Decode)]
pub struct Ports {
    pub rpc_port: u16,
    pub p_2_p_port: u16,
}

#[cfg(target_arch = "wasm32")]
impl OpfsRedbWorker {
    async fn new(file_url: &str) -> Result<Self, anyhow::Error> {
        let db = Database::create(file_url)?;

        // Initialize tables
        let write_txn = db.begin_write()?;
        {
            write_txn.open_table(USER_ACCOUNT_TABLE)?;
            write_txn.open_table(PORT_TABLE)?;
            write_txn.open_table(TRANSACTIONS_DATA_TABLE)?;
            write_txn.open_table(TRANSACTION_TABLE)?;
            write_txn.open_table(NONCE_TABLE)?;
            write_txn.open_table(USER_PEER_TABLE)?;
            write_txn.open_table(SAVED_PEERS_TABLE)?;
        }
        write_txn.commit()?;

        Ok(Self { db })
    }
}



#[cfg(target_arch = "wasm32")]
impl DbWorkerInterface for OpfsRedbWorker {
    async fn initialize_db_client(file_url: &str) -> Result<Self, anyhow::Error> {
        Self::new(file_url).await
    }

    async fn set_user_account(&self, user: UserAccount) -> Result<(), anyhow::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(USER_ACCOUNT_TABLE)?;
            let user_data = user.encode();
            table.insert(USER_ACC_KEY,user_data);
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn get_nonce(&self) -> Result<u32, anyhow::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(NONCE_TABLE)?;
        Ok(table.get(&NONCE_KEY)?.map(|v| v.value()).unwrap_or(0))
    }

    async fn increment_nonce(&self) -> Result<(), anyhow::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(NONCE_TABLE)?;
            let current = table.get(&NONCE_KEY)?.map(|v| v.value()).unwrap_or(0);
            table.insert(&NONCE_KEY, &(current + 1))?;
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn update_success_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut tx_table = write_txn.open_table(TRANSACTION_TABLE)?;
            let mut data_table = write_txn.open_table(TRANSACTIONS_DATA_TABLE)?;

            // Update transaction
            let tx_data = tx_state.encode();
            let to_store = if let Some(get_txs) = tx_table.get(TXS_KEY).map_err(|err|anyhow!("error on txs:{err:?}"))?{
                let mut saved_txs = get_txs.value();
                saved_txs.push(tx_data);
                saved_txs
            }else{
                vec![]
            };
            tx_table.insert(TXS_KEY, to_store)?;

            // Update total success value
            let current_data = data_table.get(TXS_DATA_KEY)?
                .map(|v|{
                    let val = v.value();
                    let decoded_value:TransactionsData = Decode::decode(&mut &val[..]).expect("failed to decode");
                    decoded_value
                })
                .unwrap_or(TransactionsData { success_value: 0, failed_value: 0 });

            let new_data = TransactionsData {
                success_value: current_data.success_value + tx_state.amount as i64,
                ..current_data
            };
            let val_new_data = new_data.encode();
            data_table.insert(TXS_DATA_KEY, &val_new_data)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn get_user_accounts(
        &self,
        network: ChainSupported,
    ) -> Result<Vec<UserAccount>, anyhow::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(USER_ACCOUNT_TABLE)?;

        let mut accounts = Vec::new();
        for result in table.iter()? {
            let (_, value) = result?;
            let account: UserAccount = Decode::decode(&mut &value.value()[..]).map_err(|err|anyhow!("failed to decode: {err:?}"))?;
            if account.network == network {
                accounts.push(account);
            }
        }
        Ok(accounts)
    }

    async fn update_failed_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut tx_table = write_txn.open_table(TRANSACTION_TABLE)?;
            let mut data_table = write_txn.open_table(TRANSACTIONS_DATA_TABLE)?;

            // Update transaction
            let tx_data = tx_state.encode();
            let to_store = if let Some(get_txs) = tx_table.get(TXS_KEY).map_err(|err|anyhow!("error on txs:{err:?}"))?{
                let mut saved_txs = get_txs.value();
                saved_txs.push(tx_data);
                saved_txs
            }else{
                vec![]
            };
            tx_table.insert(TXS_KEY, to_store)?;

            // Update total failed value
            let current_data = data_table.get(TXS_DATA_KEY)?
                .map(|v| {
                   let val = v.value();
                   let decoded_val: TransactionsData = Decode::decode(&mut &val[..]).expect("failed to decode");
                    decoded_val
                })
                .unwrap_or(TransactionsData { success_value: 0, failed_value: 0 });

            let new_data = TransactionsData {
                failed_value: current_data.failed_value + tx_state.amount as i64,
                ..current_data
            };

            data_table.insert(TXS_DATA_KEY, &new_data.encode())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn get_failed_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TRANSACTION_TABLE)?;

        let mut failed_txs = Vec::new();
        let values = table.get(TXS_KEY).map_err(|err|anyhow!("failed to get failed_txs: {err:?}"))?.expect("failed to get failed txs");
        for value in values.value() {
            let tx: DbTxStateMachine = Decode::decode(&mut &value[..]).map_err(|err|anyhow!("failed to decode: {err:?}"))?;
            if !tx.success {
                failed_txs.push(tx);
            }
        }
        Ok(failed_txs)
    }

    async fn get_success_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TRANSACTION_TABLE)?;

        let mut success_txs = Vec::new();
        let values = table.get(TXS_KEY).map_err(|err|anyhow!("failed to get success_txs: {err:?}"))?.expect("failed to get success txs");
        for value in values.value() {
            let tx: DbTxStateMachine = Decode::decode(&mut &value[..]).map_err(|err|anyhow!("failed to decode: {err:?}"))?;
            if tx.success {
                success_txs.push(tx);
            }
        }
        Ok(success_txs)
    }

    async fn record_user_peer_id(&self, peer_record: PeerRecord) -> Result<(), anyhow::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(USER_PEER_TABLE)?;
            let peer_data = peer_record.encode();
            table.insert(USER_PEER_RECORD_KEY, &peer_data)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn get_user_peer_id(
        &self,
        account_id: Option<String>,
        peer_id: Option<String>,
    ) -> Result<PeerRecord, anyhow::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(USER_PEER_TABLE)?;
        if let Some(value) = table.get(USER_PEER_RECORD_KEY)? {
            let peer: PeerRecord = Decode::decode(&mut &value.value()[..]).map_err(|err| anyhow!("failed to decode: {err:?}"))?;

            if let Some(ref acc_id) = account_id {
                if peer.account_id1.as_ref().unwrap() == acc_id {
                    return Ok(peer.clone());
                }
            }

            if let Some(ref pid) = peer_id {
                if peer.peer_id.as_ref().unwrap() == pid {
                    return Ok(peer.clone());
                }
            }
        }
        Err(anyhow!("Peer not found"))
    }

    async fn set_ports(&self, rpc: u16, p2p: u16) -> Result<(), anyhow::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(PORT_TABLE)?;
            let ports = Ports {
                rpc_port: rpc,
                p_2_p_port:p2p
            };
            let port_data = ports.encode();
            table.insert(PORTS_KEY, &port_data)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn get_ports(&self) -> Result<Option<Ports>, anyhow::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(PORT_TABLE)?;

        if let Some(value) = table.get(PORTS_KEY)? {
            let ports: Ports = Decode::decode(&mut &value.value()[..]).map_err(|err|anyhow!("failed to decode: {err:?}"))?;
            Ok(Some(ports))
        } else {
            Ok(None)
        }
    }

    async fn get_total_value_success(&self) -> Result<u64, anyhow::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TRANSACTIONS_DATA_TABLE)?;

        let data = table.get(TXS_DATA_KEY)?
            .map(|v|{
                let decoded_val:TransactionsData = Decode::decode(&mut &v.value()[..]).expect("failed to decode");
                decoded_val
            })
            .unwrap_or(TransactionsData { success_value: 0, failed_value: 0 });

        Ok(data.success_value as u64)
    }

    async fn get_total_value_failed(&self) -> Result<u64, anyhow::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TRANSACTIONS_DATA_TABLE)?;

        let data = table.get(TXS_DATA_KEY)?
            .map(|v|{
                let decoded_val:TransactionsData = Decode::decode(&mut &v.value()[..]).expect("failed to decode");
                decoded_val
            })
            .unwrap_or(TransactionsData { success_value: 0, failed_value: 0 });

        Ok(data.failed_value as u64)
    }

    async fn update_user_peer_id_accounts(&self, peer_record: PeerRecord) -> Result<(), Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(USER_PEER_TABLE)?;

            // Get current data
            let encoded_to_store ={
                let val = table.get(&USER_PEER_RECORD_KEY).map_err(|err|anyhow!("failed to get user peer record: {err:?}"))?.expect("user not available");
                let mut current_peer: PeerRecord = Decode::decode(&mut &val.value()[..]).map_err(|err| anyhow!("failed to decode: {err:?}"))?;

                // Update account IDs if provided
                if let Some(account_id) = peer_record.account_id1 {
                    current_peer.account_id1 = Some(account_id);
                }
                if let Some(account_id) = peer_record.account_id2 {
                    current_peer.account_id2 = Some(account_id);
                }
                if let Some(account_id) = peer_record.account_id3 {
                    current_peer.account_id3 = Some(account_id);
                }
                if let Some(account_id) = peer_record.account_id4 {
                    current_peer.account_id4 = Some(account_id);
                }
                current_peer.encode()
            };
            // Save updated data
            table.insert(SAVED_PEERS_KEY, encoded_to_store)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn record_saved_user_peers(&self, peer_record: PeerRecord) -> Result<(), Error> {
        let write_txn = self.db.begin_write()?;
        {
            let encoded_data = peer_record.encode();
            let mut table = write_txn.open_table(SAVED_PEERS_TABLE)?;
            let to_store:Vec<Vec<u8>> = if let Some(get_saved_peers) = table.get(SAVED_PEERS_KEY).map_err(|err|anyhow!("error on saved peers:{err:?}"))?{
                let mut saved_peers = get_saved_peers.value();
                saved_peers.push(encoded_data);
                saved_peers
            }else{
                vec![]
            };
            table.insert(SAVED_PEERS_KEY, to_store)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn get_saved_user_peers(&self, account_id: String) -> Result<PeerRecord, Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(SAVED_PEERS_TABLE)?;

        let saved_peers = table.get(SAVED_PEERS_KEY).map_err(|err|anyhow!("failed to get saved peer record: {err:?}"))?.expect("saved peers not available");
        for value in saved_peers.value() {
            let peer:PeerRecord = Decode::decode(&mut &value[..]).map_err(|err|anyhow!("failed to decode: {err:?}"))?;

            // Check all account ID fields
            if peer.account_id1 == Some(account_id.clone()) ||
                peer.account_id2 == Some(account_id.clone()) ||
                peer.account_id3 == Some(account_id.clone()) ||
                peer.account_id4 == Some(account_id.clone()) {
                return Ok(peer);
            }
        }

        Err(anyhow!("No saved peer found for account ID: {}", account_id))
    }
}

/// Handling connection and interaction with the local database
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
pub struct LocalDbWorker {
    db: Arc<PrismaClient>,
}

#[cfg(not(target_arch = "wasm32"))]
const SERVER_DATA_ID: i32 = 1;

#[cfg(not(target_arch = "wasm32"))]
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
    ) -> Result<Vec<user_account::Data>, anyhow::Error> {
        let accounts = self
            .db
            .user_account()
            .find_many(vec![user_account::WhereParam::NetworkId(
                StringFilter::Equals(network.into()),
            )])
            .exec()
            .await?;
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

    async fn get_failed_txs(&self) -> Result<Vec<transaction::Data>, anyhow::Error> {
        let failed_txs = self
            .db
            .transaction()
            .find_many(vec![transaction::WhereParam::Status(BoolFilter::Equals(
                false,
            ))])
            .exec()
            .await?;
        Ok(failed_txs)
    }

    async fn get_success_txs(&self) -> Result<Vec<transaction::Data>, anyhow::Error> {
        let success_txs = self
            .db
            .transaction()
            .find_many(vec![transaction::WhereParam::Status(BoolFilter::Equals(
                true,
            ))])
            .exec()
            .await?;
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
        self.db
            .user_peer()
            .create(
                peer_record.record_id,
                peer_record.peer_id.unwrap(),
                peer_record.account_id1.unwrap_or("".to_string()),
                peer_record.account_id2.unwrap_or("".to_string()),
                peer_record.account_id3.unwrap_or("".to_string()),
                peer_record.account_id4.unwrap_or("".to_string()),
                peer_record.multi_addr.unwrap(),
                peer_record.keypair.unwrap(),
                Default::default(),
            )
            .exec()
            .await?;
        Ok(())
    }

    async fn update_user_peer_id_accounts(
        &self,
        peer_record: PeerRecord,
    ) -> Result<(), anyhow::Error> {
        // Create a vector to collect the update futures
        let mut batch_updates = Vec::new();

        // Check and push updates for each account ID
        if let Some(account_id) = peer_record.account_id1 {
            let update_future = self.db.user_peer().update(
                user_peer::id::equals(1),
                vec![user_peer::account_id_1::set(account_id)],
            );
            batch_updates.push(update_future);
        }

        if let Some(account_id) = peer_record.account_id2 {
            let update_future = self.db.user_peer().update(
                user_peer::id::equals(1),
                vec![user_peer::account_id_2::set(account_id)],
            );
            batch_updates.push(update_future);
        }

        if let Some(account_id) = peer_record.account_id3 {
            let update_future = self.db.user_peer().update(
                user_peer::id::equals(1),
                vec![user_peer::account_id_3::set(account_id)],
            );
            batch_updates.push(update_future);
        }

        if let Some(account_id) = peer_record.account_id4 {
            let update_future = self.db.user_peer().update(
                user_peer::id::equals(1),
                vec![user_peer::account_id_4::set(account_id)],
            );
            batch_updates.push(update_future);
        }

        // Execute all updates in a batch
        self.db._batch(batch_updates).await?;
        Ok(())
    }

    // get peer by account id by either account id or peerId
    async fn get_user_peer_id(
        &self,
        account_id: Option<String>,
        peer_id: Option<String>,
    ) -> Result<user_peer::Data, anyhow::Error> {
        let where_param = match (account_id, peer_id) {
            (Some(acc_id), _) => user_peer::WhereParam::AccountId1(StringFilter::Equals(acc_id)),
            (_, Some(pid)) => user_peer::WhereParam::PeerId(StringFilter::Equals(pid)),
            (None, None) => return Err(anyhow!("Please provide either account ID or peer ID")),
        };

        self.db
            .user_peer()
            .find_first(vec![where_param])
            .exec()
            .await?
            .ok_or_else(|| anyhow!("Peer not found in DB"))
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
    async fn get_ports(&self) -> Result<Option<port::Data>, anyhow::Error> {
        let ports = self
            .db
            .port()
            .find_unique(port::UniqueWhereParam::IdEquals(1))
            .exec()
            .await?;
        Ok(ports)
    }

    // saved peers interacted with
    async fn record_saved_user_peers(
        &self,
        peer_record: PeerRecord,
    ) -> Result<(), anyhow::Error> {
        self.db
            .saved_peers()
            .create(
                peer_record.peer_id.unwrap(),
                peer_record.account_id1.unwrap(),
                peer_record.account_id2.unwrap_or("".to_string()),
                peer_record.account_id3.unwrap_or("".to_string()),
                peer_record.account_id4.unwrap_or("".to_string()),
                peer_record.multi_addr.unwrap(),
                Default::default(),
            )
            .exec()
            .await?;
        Ok(())
    }

    // get saved peers
    async fn get_saved_user_peers(
        &self,
        account_id: String,
    ) -> Result<saved_peers::Data, anyhow::Error> {
        let peer_data = self
            .db
            .saved_peers()
            .find_first(vec![saved_peers::WhereParam::AccountId1(
                StringFilter::Equals(account_id),
            )])
            .exec()
            .await?
            .ok_or(anyhow!("Peer Not found in DB"))?;
        Ok(peer_data)
    }
}

// Type convertions
#[cfg(not(target_arch = "wasm32"))]
impl From<user_peer::Data> for PeerRecord {
    fn from(value: user_peer::Data) -> Self {
        Self {
            record_id: value.record_id,
            peer_id: Some(value.peer_id),
            account_id1: Some(value.account_id_1),
            account_id2: None,
            account_id3: None,
            account_id4: None,
            multi_addr: Some(value.multi_addr),
            keypair: Some(value.keypair),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<saved_peers::Data> for PeerRecord {
    fn from(value: saved_peers::Data) -> Self {
        Self {
            record_id: "".to_string(),
            peer_id: Some(value.node_id),
            account_id1: Some(value.account_id_1),
            account_id2: None,
            account_id3: None,
            account_id4: None,
            multi_addr: Some(value.multi_addr),
            keypair: None,
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
