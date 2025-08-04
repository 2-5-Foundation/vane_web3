
use anyhow::{anyhow, Error};
use codec::{Decode, Encode};
use primitives::data_structure::{
    ChainSupported, DbTxStateMachine, DbWorkerInterface, PeerRecord, Ports, UserAccount, AccountInfo
};
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use web_sys::{FileSystemDirectoryHandle, StorageManager};

// ======================================= Define table schemas =============================== //
const USER_ACCOUNT_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("user_accounts");
const PORT_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("ports");
const TRANSACTIONS_DATA_TABLE: TableDefinition<&str, Vec<u8>> =
    TableDefinition::new("transactions_data");
// stores array of tx but all are encoded
const TRANSACTION_TABLE: TableDefinition<&str, Vec<Vec<u8>>> = TableDefinition::new("transactions");
const NONCE_TABLE: TableDefinition<&str, u32> = TableDefinition::new("nonce");
// stores array of user profiles
const USER_PEER_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("user_peers");

// stores array of saved peers
const SAVED_PEERS_TABLE: TableDefinition<&str, Vec<Vec<u8>>> = TableDefinition::new("saved_peers");

// ===================================== DB KEYS ====================================== //
pub const USER_ACC_KEY: &str = "user_account";
pub const NONCE_KEY: &str = "nonce_key";
pub const TXS_KEY: &str = "txs_key";
pub const TXS_DATA_KEY: &str = "txs_data_key";
pub const USER_PEER_RECORD_KEY: &str = "user_peer";
pub const SAVED_PEERS_KEY: &str = "saved_peers";
pub const PORTS_KEY: &str = "saved_ports";

/// handling connection and interaction with the browser based OPFS database
pub struct OpfsRedbWorker {
    db: Database,
}

// ============================== REDB SCHEMA ================================ //
#[derive(Serialize, Deserialize, Encode, Decode)]
struct TransactionsData {
    success_value: i64,
    failed_value: i64,
}

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

impl DbWorkerInterface for OpfsRedbWorker {
    async fn initialize_db_client(file_url: &str) -> Result<Self, anyhow::Error> {
        Self::new(file_url).await
    }

    async fn set_user_account(&self, user: UserAccount) -> Result<(), anyhow::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(USER_ACCOUNT_TABLE)?;
            let user_data = user.encode();
            table.insert(USER_ACC_KEY, user_data);
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
            let to_store = if let Some(get_txs) = tx_table
                .get(TXS_KEY)
                .map_err(|err| anyhow!("error on txs:{err:?}"))?
            {
                let mut saved_txs = get_txs.value();
                saved_txs.push(tx_data);
                saved_txs
            } else {
                vec![]
            };
            tx_table.insert(TXS_KEY, to_store)?;

            // Update total success value
            let current_data = data_table
                .get(TXS_DATA_KEY)?
                .map(|v| {
                    let val = v.value();
                    let decoded_value: TransactionsData =
                        Decode::decode(&mut &val[..]).expect("failed to decode");
                    decoded_value
                })
                .unwrap_or(TransactionsData {
                    success_value: 0,
                    failed_value: 0,
                });

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
            let account: UserAccount = Decode::decode(&mut &value.value()[..])
                .map_err(|err| anyhow!("failed to decode: {err:?}"))?;
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
            let to_store = if let Some(get_txs) = tx_table
                .get(TXS_KEY)
                .map_err(|err| anyhow!("error on txs:{err:?}"))?
            {
                let mut saved_txs = get_txs.value();
                saved_txs.push(tx_data);
                saved_txs
            } else {
                vec![]
            };
            tx_table.insert(TXS_KEY, to_store)?;

            // Update total failed value
            let current_data = data_table
                .get(TXS_DATA_KEY)?
                .map(|v| {
                    let val = v.value();
                    let decoded_val: TransactionsData =
                        Decode::decode(&mut &val[..]).expect("failed to decode");
                    decoded_val
                })
                .unwrap_or(TransactionsData {
                    success_value: 0,
                    failed_value: 0,
                });

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
        let values = table
            .get(TXS_KEY)
            .map_err(|err| anyhow!("failed to get failed_txs: {err:?}"))?
            .expect("failed to get failed txs");
        for value in values.value() {
            let tx: DbTxStateMachine = Decode::decode(&mut &value[..])
                .map_err(|err| anyhow!("failed to decode: {err:?}"))?;
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
        let values = table
            .get(TXS_KEY)
            .map_err(|err| anyhow!("failed to get success_txs: {err:?}"))?
            .expect("failed to get success txs");
        for value in values.value() {
            let tx: DbTxStateMachine = Decode::decode(&mut &value[..])
                .map_err(|err| anyhow!("failed to decode: {err:?}"))?;
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
        // let read_txn = self.db.begin_read()?;
        // let table = read_txn.open_table(USER_PEER_TABLE)?;
        // if let Some(value) = table.get(USER_PEER_RECORD_KEY)? {
        //     let peer: PeerRecord = Decode::decode(&mut &value.value()[..])
        //         .map_err(|err| anyhow!("failed to decode: {err:?}"))?;

        //     if let Some(ref acc_id) = account_id {
        //         if peer.account_id1.as_ref().unwrap() == acc_id {
        //             return Ok(peer.clone());
        //         }
        //     }

        //     if let Some(ref pid) = peer_id {
        //         if peer.peer_id.as_ref().unwrap() == pid {
        //             return Ok(peer.clone());
        //         }
        //     }
        // }
        // Err(anyhow!("Peer not found"))
        todo!()
    }

    async fn set_ports(&self, _rpc: u16, p2p: u16) -> Result<(), anyhow::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(PORT_TABLE)?;
            let ports = Ports {
                rpc: p2p,
                p_2_p_port: p2p,
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
            let ports: Ports = Decode::decode(&mut &value.value()[..])
                .map_err(|err| anyhow!("failed to decode: {err:?}"))?;
            Ok(Some(ports))
        } else {
            Ok(None)
        }
    }

    async fn get_total_value_success(&self) -> Result<u64, anyhow::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TRANSACTIONS_DATA_TABLE)?;

        let data = table
            .get(TXS_DATA_KEY)?
            .map(|v| {
                let decoded_val: TransactionsData =
                    Decode::decode(&mut &v.value()[..]).expect("failed to decode");
                decoded_val
            })
            .unwrap_or(TransactionsData {
                success_value: 0,
                failed_value: 0,
            });

        Ok(data.success_value as u64)
    }

    async fn get_total_value_failed(&self) -> Result<u64, anyhow::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TRANSACTIONS_DATA_TABLE)?;

        let data = table
            .get(TXS_DATA_KEY)?
            .map(|v| {
                let decoded_val: TransactionsData =
                    Decode::decode(&mut &v.value()[..]).expect("failed to decode");
                decoded_val
            })
            .unwrap_or(TransactionsData {
                success_value: 0,
                failed_value: 0,
            });

        Ok(data.failed_value as u64)
    }

    async fn update_user_peer_id_account_ids(&self, peer_record: AccountInfo) -> Result<(), Error> {
        todo!()
    }

    async fn record_saved_user_peers(&self, peer_record: PeerRecord) -> Result<(), Error> {
        let write_txn = self.db.begin_write()?;
        {
            let encoded_data = peer_record.encode();
            let mut table = write_txn.open_table(SAVED_PEERS_TABLE)?;
            let to_store: Vec<Vec<u8>> = if let Some(get_saved_peers) =
                table
                    .get(SAVED_PEERS_KEY)
                    .map_err(|err| anyhow!("error on saved peers:{err:?}"))?
            {
                let mut saved_peers = get_saved_peers.value();
                saved_peers.push(encoded_data);
                saved_peers
            } else {
                vec![]
            };
            table.insert(SAVED_PEERS_KEY, to_store)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn get_saved_user_peers(&self, account_id: String) -> Result<PeerRecord, Error> {
        // let read_txn = self.db.begin_read()?;
        // let table = read_txn.open_table(SAVED_PEERS_TABLE)?;

        // let saved_peers = table
        //     .get(SAVED_PEERS_KEY)
        //     .map_err(|err| anyhow!("failed to get saved peer record: {err:?}"))?
        //     .expect("saved peers not available");
        // for value in saved_peers.value() {
        //     let peer: PeerRecord = Decode::decode(&mut &value[..])
        //         .map_err(|err| anyhow!("failed to decode: {err:?}"))?;

        //     // Check all account ID fields
        //     if peer.account_id1 == Some(account_id.clone())
        //         || peer.account_id2 == Some(account_id.clone())
        //         || peer.account_id3 == Some(account_id.clone())
        //         || peer.account_id4 == Some(account_id.clone())
        //     {
        //         return Ok(peer);
        //     }
        // }

        // Err(anyhow!(
        //     "No saved peer found for account ID: {}",
        //     account_id
        // ))
        todo!()
    }
}
