use anyhow::anyhow;
use jsonrpsee::{
    core::{async_trait, RpcResult, SubscriptionResult},
    proc_macros::rpc,
    PendingSubscriptionSink, SubscriptionMessage,
};
use log::{debug, trace};
use std::sync::Arc;

#[rpc(server, client)]
pub trait RelayServerRpc {
    /// watch relay network status
    #[subscription(name ="subscribeRelayNetworkStatus",item = String )]
    async fn watch_relay_network_status(&self) -> SubscriptionResult;
}

pub struct RelayServerRpcWorker {}

impl RelayServerRpcWorker {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl RelayServerRpcServer for RelayServerRpcWorker {
    async fn watch_relay_network_status(
        &self,
        subscription_sink: PendingSubscriptionSink,
    ) -> SubscriptionResult {
        let sink = subscription_sink
            .accept()
            .await
            .map_err(|_| anyhow!("failed to accept rpc ws channel"))?;
        Ok(())
    }
}
