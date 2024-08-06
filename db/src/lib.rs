// store user txs
// failed and reverted txs
// peers connected as cache
// store value of each txs ( reverted and succeeded )

extern crate alloc;

mod db;

#[cfg(test)]
mod tests;

use alloc::sync::Arc;
use crate::db::{
    new_client_with_url,
    read_filters::{IntFilter, StringFilter},
    PrismaClient, PrismaClientBuilder,
};
use prisma_client_rust::{query_core::RawQuery, BatchItem, Direction, PrismaValue, Raw};
use serde::{Deserialize, Serialize};


/// Handling connection and interaction with the database
pub struct DbWorker {
    db: Arc<PrismaClient>
}

impl DbWorker {

    pub fn record_tx(&self) -> Result<(), anyhow::Error>{
        Ok(())
    }

    pub fn set_user(&self) -> Result<(),anyhow::Error> {
        Ok(())
    }

    pub fn update_user_account(&self) -> Result<(),anyhow::Error>{
        Ok(())
    }

    pub fn update_success_tx(&self) -> Result<(), anyhow::Error>{
        Ok(())
    }

    pub fn update_failed_tx(&self) -> Result<(), anyhow::Error>{
        Ok(())
    }

    pub fn record_peer(&self) -> Result<(), anyhow::Error>{
        Ok(())
    }

    pub fn get_failed_tx_stream(&self) -> Result<(), anyhow::Error>{
        todo!();
        Ok(())
    }

    pub fn get_success_tx_stream(&self) -> Result<(), anyhow::Error>{
        todo!()
    }

    pub fn get_total_value_success(&self) -> Result<u64,anyhow::Error>{
        todo!()
    }

    pub fn get_total_value_failed(&self) -> Result<u64,anyhow::Error>{
        todo!()
    }

    pub fn get_saved_peers(&self) -> Result<(), anyhow::Error>{
        todo!()
    }

}