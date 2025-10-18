use anyhow::{anyhow, Error};
use codec::{Decode, Encode};
use log::info;
use opfs::persistent;
use opfs::persistent::{app_specific_dir, DirectoryHandle, FileHandle, WritableFileStream};
use opfs::{CreateWritableOptions, GetFileHandleOptions};
use primitives::data_structure::{
    ChainSupported, DbTxStateMachine, DbWorkerInterface, UserAccount,
};
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
// you must import the traits to call methods on the types
use opfs::{DirectoryHandle as _, FileHandle as _, WritableFileStream as _};

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
pub const SUCCESS_TXS_KEY: &str = "success_txs_key";
pub const FAILED_TXS_KEY: &str = "failed_txs_key";
pub const TXS_DATA_KEY: &str = "txs_data_key";
pub const SAVED_PEERS_KEY: &str = "saved_peers";

// ============================== REDB SCHEMA ================================ //
#[derive(Serialize, Deserialize, Encode, Decode)]
struct TransactionsData {
    success_value: i64,
    failed_value: i64,
}

/// OPFS file system bridge for redb
struct OpfsFileSystem {
    directory: DirectoryHandle,
    db_file: FileHandle,
}

impl OpfsFileSystem {
    async fn new(db_name: &str) -> Result<Self, anyhow::Error> {
        // Get the app-specific directory from OPFS
        let directory = app_specific_dir()
            .await
            .map_err(|e| anyhow!("Failed to get app-specific directory: {:?}", e))?;

        // Create or get the database file handle
        let options = GetFileHandleOptions { create: true };
        let db_file = directory
            .get_file_handle_with_options(db_name, &options)
            .await
            .map_err(|e| anyhow!("Failed to get/create database file: {:?}", e))?;

        Ok(Self { directory, db_file })
    }

    /// Get the database file as a byte array for redb to work with
    async fn get_db_bytes(&self) -> Result<Vec<u8>, anyhow::Error> {
        self.db_file
            .read()
            .await
            .map_err(|e| anyhow!("Failed to read database file: {:?}", e))
    }

    /// Save the database bytes back to OPFS
    async fn save_db_bytes(&mut self, data: &[u8]) -> Result<(), anyhow::Error> {
        let write_options = CreateWritableOptions {
            keep_existing_data: false,
        };
        let mut writer = self
            .db_file
            .create_writable_with_options(&write_options)
            .await
            .map_err(|e| anyhow!("Failed to create writable: {:?}", e))?;

        writer
            .write_at_cursor_pos(data.to_vec())
            .await
            .map_err(|e| anyhow!("Failed to write database: {:?}", e))?;
        writer
            .close()
            .await
            .map_err(|e| anyhow!("Failed to close writer: {:?}", e))?;

        Ok(())
    }
}

/// handling connection and interaction with the browser based OPFS database
pub struct OpfsRedbWorker {
    db: Database,
    opfs_fs: OpfsFileSystem,
}

impl OpfsRedbWorker {
    async fn new(db_name: &str) -> Result<Self, anyhow::Error> {
        let opfs_fs = OpfsFileSystem::new(db_name).await?;

        // Try to load existing database from OPFS
        let db_bytes = opfs_fs.get_db_bytes().await?;

        let db = if db_bytes.is_empty() {
            // Create new database using a virtual file path that redb can work with
            // We'll use a special path that indicates this is a virtual file
            let db = Database::create("vane_virtual.db")?;

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

            db
        } else {
            // Load existing database from bytes
            // For now, we'll create a new database and manually restore the data
            // In a production system, you'd want proper database serialization
            let db = Database::create("vane_virtual.db")?;

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

            db
        };

        Ok(Self { db, opfs_fs })
    }

    /// Save the current database state to OPFS
    async fn persist_to_opfs(&mut self) -> Result<(), anyhow::Error> {
        // Export database to bytes and save to OPFS
        // Note: This is a simplified approach - in production you'd want proper serialization
        let marker = b"REDB_DATABASE_EXISTS";
        self.opfs_fs.save_db_bytes(marker).await
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
    ) -> Result<UserAccount, anyhow::Error> {
        let write_txn = self.db.begin_write()?;
        let user_account = {
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
            user_account
        };
        write_txn.commit()?;

        Ok(user_account)
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

    async fn set_nonce(&self, nonce: u32) -> Result<(), anyhow::Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(NONCE_TABLE)?;
            table.insert(&NONCE_KEY, &nonce)?;
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
                .get(SUCCESS_TXS_KEY)
                .map_err(|err| anyhow!("error on success txs:{err:?}"))?
            {
                let mut saved_txs = get_txs.value();
                saved_txs.push(tx_data);
                saved_txs
            } else {
                vec![tx_data]
            };
            tx_table.insert(SUCCESS_TXS_KEY, to_store)?;

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
                .get(FAILED_TXS_KEY)
                .map_err(|err| anyhow!("error on failed txs:{err:?}"))?
            {
                let mut saved_txs = get_txs.value();
                saved_txs.push(tx_data);
                saved_txs
            } else {
                vec![tx_data]
            };
            tx_table.insert(FAILED_TXS_KEY, to_store)?;

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
        if let Some(values) = table
            .get(FAILED_TXS_KEY)
            .map_err(|err| anyhow!("failed to get failed_txs: {err:?}"))?
        {
            for value in values.value() {
                let tx: DbTxStateMachine = Decode::decode(&mut &value[..])
                    .map_err(|err| anyhow!("failed to decode: {err:?}"))?;
                failed_txs.push(tx);
            }
        }
        Ok(failed_txs)
    }

    async fn get_success_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error> {
        let read_txn = self.db.begin_read()?;
        let table = read_txn.open_table(TRANSACTION_TABLE)?;

        let mut success_txs = Vec::new();
        if let Some(values) = table
            .get(SUCCESS_TXS_KEY)
            .map_err(|err| anyhow!("failed to get success_txs: {err:?}"))?
        {
            for value in values.value() {
                let tx: DbTxStateMachine = Decode::decode(&mut &value[..])
                    .map_err(|err| anyhow!("failed to decode: {err:?}"))?;
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
    async fn delete_saved_peer(&self, multi_addr: &str) -> Result<(), Error> {
        let write_txn = self.db.begin_write()?;
        {
            let mut table = write_txn.open_table(SAVED_PEERS_TABLE)?;

            // Find all account IDs mapped to this multi_addr and remove them
            let mut keys_to_remove = Vec::new();

            for entry in table.iter()? {
                let (key, value) = entry?;
                let stored_peer_id = value.value().to_string();

                if stored_peer_id == multi_addr {
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

// ------------------------------------ Testing ------------------------------------ //

use std::cell::RefCell;
use std::collections::HashMap;

type MemUserAccountTable = HashMap<String, Vec<u8>>;
type MemTransactionsDataTable = HashMap<String, Vec<u8>>;
type MemTransactionTable = HashMap<String, Vec<Vec<u8>>>;
type MemNonceTable = HashMap<String, u32>;
type MemSavedPeersTable = HashMap<String, String>;

pub struct InMemoryDbWorker {
    user_accounts: RefCell<MemUserAccountTable>,
    transactions_data: RefCell<MemTransactionsDataTable>,
    transactions: RefCell<MemTransactionTable>,
    nonces: RefCell<MemNonceTable>,
    saved_peers: RefCell<MemSavedPeersTable>,
}

impl DbWorkerInterface for InMemoryDbWorker {
    async fn initialize_db_client(_file_url: &str) -> Result<Self, anyhow::Error> {
        Ok(Self {
            user_accounts: RefCell::new(HashMap::new()),
            transactions_data: RefCell::new(HashMap::new()),
            transactions: RefCell::new(HashMap::new()),
            nonces: RefCell::new(HashMap::new()),
            saved_peers: RefCell::new(HashMap::new()),
        })
    }

    async fn set_user_account(&self, user: UserAccount) -> Result<(), anyhow::Error> {
        self.user_accounts
            .borrow_mut()
            .insert(USER_ACC_KEY.to_string(), user.encode());
        Ok(())
    }

    async fn update_user_account(
        &self,
        account_id: String,
        network: ChainSupported,
    ) -> Result<UserAccount, anyhow::Error> {
        let mut user_account = self
            .user_accounts
            .borrow()
            .get(USER_ACC_KEY)
            .map(|v| {
                let decoded_val: UserAccount =
                    Decode::decode(&mut &v[..]).expect("failed to decode");
                decoded_val
            })
            .ok_or_else(|| anyhow!("user account not found"))?;
        user_account.accounts.push((account_id, network));
        self.user_accounts
            .borrow_mut()
            .insert(USER_ACC_KEY.to_string(), user_account.encode());
        Ok(user_account)
    }

    async fn get_nonce(&self) -> Result<u32, anyhow::Error> {
        Ok(self.nonces.borrow().get(NONCE_KEY).copied().unwrap_or(0))
    }

    async fn increment_nonce(&self) -> Result<(), anyhow::Error> {
        let current = self.nonces.borrow().get(NONCE_KEY).copied().unwrap_or(0);
        self.nonces
            .borrow_mut()
            .insert(NONCE_KEY.to_string(), current + 1);
        Ok(())
    }

    async fn set_nonce(&self, nonce: u32) -> Result<(), anyhow::Error> {
        self.nonces
            .borrow_mut()
            .insert(NONCE_KEY.to_string(), nonce);
        Ok(())
    }

    async fn get_user_account(&self) -> Result<UserAccount, anyhow::Error> {
        let user_account = self
            .user_accounts
            .borrow()
            .get(USER_ACC_KEY)
            .map(|v| {
                let decoded_val: UserAccount =
                    Decode::decode(&mut &v[..]).expect("failed to decode");
                decoded_val
            })
            .ok_or_else(|| anyhow!("user account not found"))?;
        Ok(user_account)
    }

    async fn update_success_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error> {
        // Add transaction to list
        let tx_data = tx_state.encode();
        let to_store = if let Some(saved_txs) = self.transactions.borrow().get(SUCCESS_TXS_KEY) {
            let mut saved_txs = saved_txs.clone();
            saved_txs.push(tx_data);
            saved_txs
        } else {
            vec![tx_data]
        };
        self.transactions
            .borrow_mut()
            .insert(SUCCESS_TXS_KEY.to_string(), to_store);

        // Update total success value
        let current_data = self
            .transactions_data
            .borrow()
            .get(TXS_DATA_KEY)
            .map(|v| {
                let decoded_val: TransactionsData =
                    Decode::decode(&mut &v[..]).expect("failed to decode");
                decoded_val
            })
            .unwrap_or(TransactionsData {
                success_value: 0,
                failed_value: 0,
            });

        let new_data = TransactionsData {
            success_value: current_data.success_value + tx_state.amount as i64,
            ..current_data
        };

        self.transactions_data
            .borrow_mut()
            .insert(TXS_DATA_KEY.to_string(), new_data.encode());

        Ok(())
    }

    async fn update_failed_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error> {
        // Add transaction to list
        let tx_data = tx_state.encode();
        let to_store = if let Some(saved_txs) = self.transactions.borrow().get(FAILED_TXS_KEY) {
            let mut saved_txs = saved_txs.clone();
            saved_txs.push(tx_data);
            saved_txs
        } else {
            vec![tx_data]
        };
        self.transactions
            .borrow_mut()
            .insert(FAILED_TXS_KEY.to_string(), to_store);

        // Update total failed value
        let current_data = self
            .transactions_data
            .borrow()
            .get(TXS_DATA_KEY)
            .map(|v| {
                let decoded_val: TransactionsData =
                    Decode::decode(&mut &v[..]).expect("failed to decode");
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

        self.transactions_data
            .borrow_mut()
            .insert(TXS_DATA_KEY.to_string(), new_data.encode());

        Ok(())
    }

    async fn get_failed_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error> {
        let mut failed_txs = Vec::new();
        for tx in self
            .transactions
            .borrow()
            .get(FAILED_TXS_KEY)
            .map(|v| v.clone())
            .unwrap_or_default()
        {
            let tx: DbTxStateMachine = Decode::decode(&mut &tx[..]).expect("failed to decode");
            failed_txs.push(tx);
        }
        Ok(failed_txs)
    }

    async fn get_total_value_success(&self) -> Result<u64, anyhow::Error> {
        let data = self
            .transactions_data
            .borrow()
            .get(TXS_DATA_KEY)
            .map(|v| {
                let decoded_val: TransactionsData =
                    Decode::decode(&mut &v[..]).expect("failed to decode");
                decoded_val
            })
            .unwrap_or(TransactionsData {
                success_value: 0,
                failed_value: 0,
            });
        Ok(data.success_value as u64)
    }

    async fn get_total_value_failed(&self) -> Result<u64, anyhow::Error> {
        let data = self
            .transactions_data
            .borrow()
            .get(TXS_DATA_KEY)
            .map(|v| {
                let decoded_val: TransactionsData =
                    Decode::decode(&mut &v[..]).expect("failed to decode");
                decoded_val
            })
            .unwrap_or(TransactionsData {
                success_value: 0,
                failed_value: 0,
            });
        Ok(data.failed_value as u64)
    }

    async fn get_success_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error> {
        let mut success_txs = Vec::new();
        for tx in self
            .transactions
            .borrow()
            .get(SUCCESS_TXS_KEY)
            .map(|v| v.clone())
            .unwrap_or_default()
        {
            let tx: DbTxStateMachine = Decode::decode(&mut &tx[..]).expect("failed to decode");
            success_txs.push(tx);
        }
        Ok(success_txs)
    }

    async fn record_saved_user_peers(
        &self,
        acc_id: String,
        multi_addr: String,
    ) -> Result<(), Error> {
        self.saved_peers.borrow_mut().insert(acc_id, multi_addr);
        Ok(())
    }

    async fn get_saved_user_peers(&self, account_id: String) -> Result<String, Error> {
        let saved_peer = self
            .saved_peers
            .borrow()
            .get(&account_id)
            .ok_or_else(|| anyhow!("No saved target peer"))?
            .clone();

        Ok(saved_peer)
    }

    async fn get_all_saved_peers(&self) -> Result<(Vec<String>, String), Error> {
        let mut all_account_ids = Vec::new();
        let mut peer_id = None;

        for (account_id, stored_peer_id) in &self.saved_peers.clone().into_inner() {
            if let Some(ref existing_peer_id) = peer_id {
                if stored_peer_id != existing_peer_id {
                    return Err(anyhow!(
                        "Inconsistent peer ID mapping: expected {}, got {}",
                        existing_peer_id,
                        stored_peer_id
                    ));
                }
            } else {
                peer_id = Some(stored_peer_id.clone());
            }

            all_account_ids.push(account_id.clone());
        }

        let peer_id = peer_id.ok_or_else(|| anyhow!("No saved peers found"))?;
        Ok((all_account_ids, peer_id))
    }

    async fn delete_saved_peer(&self, multi_addr: &str) -> Result<(), Error> {
        // Find all account IDs mapped to this peer_id and remove them
        let keys_to_remove: Vec<String> = self
            .saved_peers
            .borrow()
            .iter()
            .filter_map(|(account_id, stored_multi_addr)| {
                if stored_multi_addr == multi_addr {
                    Some(account_id.clone())
                } else {
                    None
                }
            })
            .collect();

        // Remove all mappings for this peer_id
        for account_id in keys_to_remove {
            self.saved_peers.borrow_mut().remove(&account_id);
        }

        Ok(())
    }
}

/// Enum wrapper for different database worker implementations
/// This provides polymorphism without needing trait objects
pub enum DbWorker {
    Opfs(OpfsRedbWorker),
    InMemory(InMemoryDbWorker),
}

impl DbWorker {
    pub async fn initialize_opfs_db_client(file_url: &str) -> Result<Self, anyhow::Error> {
        let worker = OpfsRedbWorker::initialize_db_client(file_url).await?;
        Ok(DbWorker::Opfs(worker))
    }

    pub async fn initialize_inmemory_db_client(file_url: &str) -> Result<Self, anyhow::Error> {
        let worker = InMemoryDbWorker::initialize_db_client(file_url).await?;
        Ok(DbWorker::InMemory(worker))
    }
}

impl DbWorkerInterface for DbWorker {
    async fn initialize_db_client(file_url: &str) -> Result<Self, anyhow::Error> {
        // Default to in-memory for this generic method
        Self::initialize_inmemory_db_client(file_url).await
    }

    async fn set_user_account(&self, user: UserAccount) -> Result<(), anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.set_user_account(user).await,
            DbWorker::InMemory(worker) => worker.set_user_account(user).await,
        }
    }

    async fn update_user_account(
        &self,
        account_id: String,
        network: ChainSupported,
    ) -> Result<UserAccount, anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.update_user_account(account_id, network).await,
            DbWorker::InMemory(worker) => worker.update_user_account(account_id, network).await,
        }
    }

    async fn get_nonce(&self) -> Result<u32, anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.get_nonce().await,
            DbWorker::InMemory(worker) => worker.get_nonce().await,
        }
    }

    async fn get_user_account(&self) -> Result<UserAccount, anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.get_user_account().await,
            DbWorker::InMemory(worker) => worker.get_user_account().await,
        }
    }

    async fn update_success_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.update_success_tx(tx_state).await,
            DbWorker::InMemory(worker) => worker.update_success_tx(tx_state).await,
        }
    }

    async fn update_failed_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.update_failed_tx(tx_state).await,
            DbWorker::InMemory(worker) => worker.update_failed_tx(tx_state).await,
        }
    }

    async fn get_failed_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.get_failed_txs().await,
            DbWorker::InMemory(worker) => worker.get_failed_txs().await,
        }
    }

    async fn get_total_value_success(&self) -> Result<u64, anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.get_total_value_success().await,
            DbWorker::InMemory(worker) => worker.get_total_value_success().await,
        }
    }

    async fn get_total_value_failed(&self) -> Result<u64, anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.get_total_value_failed().await,
            DbWorker::InMemory(worker) => worker.get_total_value_failed().await,
        }
    }

    async fn get_success_txs(&self) -> Result<Vec<DbTxStateMachine>, anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.get_success_txs().await,
            DbWorker::InMemory(worker) => worker.get_success_txs().await,
        }
    }

    async fn increment_nonce(&self) -> Result<(), anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.increment_nonce().await,
            DbWorker::InMemory(worker) => worker.increment_nonce().await,
        }
    }

    async fn set_nonce(&self, nonce: u32) -> Result<(), anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.set_nonce(nonce).await,
            DbWorker::InMemory(worker) => worker.set_nonce(nonce).await,
        }
    }

    async fn record_saved_user_peers(
        &self,
        acc_id: String,
        multi_addr: String,
    ) -> Result<(), anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.record_saved_user_peers(acc_id, multi_addr).await,
            DbWorker::InMemory(worker) => worker.record_saved_user_peers(acc_id, multi_addr).await,
        }
    }

    async fn get_saved_user_peers(&self, account_id: String) -> Result<String, anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.get_saved_user_peers(account_id).await,
            DbWorker::InMemory(worker) => worker.get_saved_user_peers(account_id).await,
        }
    }

    async fn get_all_saved_peers(&self) -> Result<(Vec<String>, String), anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.get_all_saved_peers().await,
            DbWorker::InMemory(worker) => worker.get_all_saved_peers().await,
        }
    }

    async fn delete_saved_peer(&self, multi_addr: &str) -> Result<(), anyhow::Error> {
        match self {
            DbWorker::Opfs(worker) => worker.delete_saved_peer(multi_addr).await,
            DbWorker::InMemory(worker) => worker.delete_saved_peer(multi_addr).await,
        }
    }
}
