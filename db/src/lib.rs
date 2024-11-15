#![allow(unused_imports)]
#![allow(unused)]
extern crate alloc;

pub mod db;

#[cfg(test)]
mod db_tests;

use crate::db::read_filters::{BoolFilter, StringFilter};
use crate::db::transactions_data::{UniqueWhereParam, WhereParam};
use crate::db::{
    new_client_with_url,
    read_filters::{BigIntFilter, BytesFilter, IntFilter},
    saved_peers, transaction, transactions_data, user_account, user_peer, PrismaClient,
    PrismaClientBuilder, UserPeerScalarFieldEnum,
};
use alloc::sync::Arc;
use anyhow::anyhow;
use codec::{Decode, Encode};
use hex;
use log::{debug, error, info, trace, warn};
use primitives::data_structure::{ChainSupported, DbTxStateMachine, PeerRecord, UserAccount};
use prisma_client_rust::{query_core::RawQuery, BatchItem, Direction, PrismaValue, Raw};
use serde::{Deserialize, Serialize};
use std::future::Future;

/// Handling connection and interaction with the database
#[derive(Clone)]
pub struct DbWorker {
    db: Arc<PrismaClient>,
}

const SERVER_DATA_ID: i32 = 1;

impl DbWorker {
    pub async fn initialize_db_client(file_url: &str) -> Result<Self, anyhow::Error> {
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

    pub async fn set_user_account(&self, user: UserAccount) -> Result<(), anyhow::Error> {
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

    // get all related network id accounts
    pub async fn get_user_accounts(
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

    pub async fn update_success_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error> {
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

    pub async fn update_failed_tx(&self, tx_state: DbTxStateMachine) -> Result<(), anyhow::Error> {
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

    pub async fn get_failed_txs(&self) -> Result<Vec<transaction::Data>, anyhow::Error> {
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

    pub async fn get_success_txs(&self) -> Result<Vec<transaction::Data>, anyhow::Error> {
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

    pub async fn get_total_value_success(&self) -> Result<u64, anyhow::Error> {
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

    pub async fn get_total_value_failed(&self) -> Result<u64, anyhow::Error> {
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

    pub async fn record_user_peer_id(&self, peer_record: PeerRecord) -> Result<(), anyhow::Error> {
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

    pub async fn update_user_peer_id_accounts(
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
    pub async fn get_user_peer_id(
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

    // saved peers interacted with
    pub async fn record_saved_user_peers(
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
    pub async fn get_saved_user_peers(
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

impl From<user_account::Data> for UserAccount {
    fn from(value: user_account::Data) -> Self {
        Self {
            user_name: value.username,
            account_id: value.account_id,
            network: ChainSupported::from(value.network_id.as_str()),
        }
    }
}

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
