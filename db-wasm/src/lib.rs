use anyhow::{anyhow, Error};
use codec::{Decode, Encode};
use primitives::data_structure::{
    AccountInfo, ChainSupported, DbTxStateMachine, DbWorkerInterface, Ports, UserAccount,
};
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use web_sys::{FileSystemDirectoryHandle, StorageManager};

// ======================================= Define table schemas =============================== //

const USER_ACCOUNT_TABLE: TableDefinition<&str, Vec<u8>> = TableDefinition::new("user_accounts");

const TRANSACTIONS_DATA_TABLE: TableDefinition<&str, Vec<u8>> =
    TableDefinition::new("transactions_data");

// stores array of tx but all are encoded
const TRANSACTION_TABLE: TableDefinition<&str, Vec<Vec<u8>>> = TableDefinition::new("transactions");

const NONCE_TABLE: TableDefinition<&str, u32> = TableDefinition::new("nonce");

// stores individual target peers accIds with multiAddr as value
const SAVED_PEERS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("saved_peers");

// ===================================== DB KEYS ====================================== //
pub const USER_ACC_KEY: &str = "user_account";
pub const NONCE_KEY: &str = "nonce_key";
pub const TXS_KEY: &str = "txs_key";
pub const TXS_DATA_KEY: &str = "txs_data_key";
pub const SAVED_PEERS_KEY: &str = "saved_peers";

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
            write_txn.open_table(TRANSACTIONS_DATA_TABLE)?;
            write_txn.open_table(TRANSACTION_TABLE)?;
            write_txn.open_table(NONCE_TABLE)?;
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
            table.insert(USER_ACC_KEY, user_data)?;
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn update_user_account(
        &self,
        account_id: String,
        network: ChainSupported,
    ) -> Result<(), anyhow::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(USER_ACCOUNT_TABLE)?;
            let mut user_account = table
                .get(USER_ACC_KEY)?
                .map(|v| {
                    let val = v.value();
                    let decoded_val: UserAccount =
                        Decode::decode(&mut &val[..]).expect("failed to decode");
                    decoded_val
                })
                .ok_or_else(|| anyhow!("user account not found"))?;
            user_account.accounts.push((account_id, network));
            table.insert(USER_ACC_KEY, &user_account.encode())?;
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

    async fn get_user_account(&self) -> Result<UserAccount, anyhow::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(USER_ACCOUNT_TABLE)?;

        let user_account = table
            .get(USER_ACC_KEY)?
            .map(|v| {
                let val = v.value();
                let decoded_val: UserAccount =
                    Decode::decode(&mut &val[..]).expect("failed to decode");
                decoded_val
            })
            .ok_or_else(|| anyhow!("user account not found"))?;
        Ok(user_account)
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

    async fn record_saved_user_peers(
        &self,
        acc_id: String,
        multi_addr: String,
    ) -> Result<(), Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(SAVED_PEERS_TABLE)?;

            // Store the account_id as key with the multi_addr as value
            // Each account maps to its multi-address
            table.insert(acc_id.as_str(), multi_addr.as_str())?;
        }
        write_txn.commit()?;
        Ok(())
    }

    async fn get_saved_user_peers(&self, account_id: String) -> Result<String, Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(SAVED_PEERS_TABLE)?;

        // Direct lookup by account_id
        if let Some(value) = table.get(account_id.as_str())? {
            Ok(value.value().to_string())
        } else {
            Err(anyhow!(
                "No saved peer found for account ID: {}",
                account_id
            ))
        }
    }

    /// Get all saved peers
    async fn get_all_saved_peers(&self) -> Result<(Vec<String>, String), Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(SAVED_PEERS_TABLE)?;
        let mut all_account_ids = Vec::new();
        let mut peer_id = None;

        // Collect all account IDs and verify they all map to the same peer ID
        for entry in table.iter()? {
            let (key, value) = entry?;
            let account_id = key.value().to_string();
            let stored_peer_id = value.value().to_string();

            // Verify all accounts map to the same peer ID
            if let Some(ref existing_peer_id) = peer_id {
                if stored_peer_id != *existing_peer_id {
                    return Err(anyhow!(
                        "Inconsistent peer ID mapping: expected {}, got {}",
                        existing_peer_id,
                        stored_peer_id
                    ));
                }
            } else {
                peer_id = Some(stored_peer_id);
            }

            all_account_ids.push(account_id);
        }

        let peer_id = peer_id.ok_or_else(|| anyhow!("No saved peers found"))?;
        Ok((all_account_ids, peer_id))
    }

    /// Delete a specific saved peer by peer_id
    async fn delete_saved_peer(&self, peer_id: &str) -> Result<(), Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(SAVED_PEERS_TABLE)?;

            // Find all account IDs mapped to this peer_id and remove them
            let mut keys_to_remove = Vec::new();

            for entry in table.iter()? {
                let (key, value) = entry?;
                let stored_peer_id = value.value().to_string();

                if stored_peer_id == peer_id {
                    let account_id = key.value().to_string();
                    keys_to_remove.push(account_id);
                }
            }

            // Remove all mappings for this peer_id
            for account_id in keys_to_remove {
                table.remove(account_id.as_str())?;
            }
        }
        write_txn.commit()?;
        Ok(())
    }
}
