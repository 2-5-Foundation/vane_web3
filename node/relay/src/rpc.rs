
use jsonrpsee::{
    PendingSubscriptionSink, SubscriptionMessage,
    core::{RpcResult, SubscriptionResult, async_trait},
    proc_macros::rpc,
};
use tokio::sync::mpsc::Receiver;
use crate::telemetry::RelayServerMetrics;
use log::trace;
use anyhow::anyhow;
use std::sync::Arc;
use tokio::sync::Mutex;

#[rpc(server, client)]
pub trait RelayServerRpc {
    /// watch relay network status
    #[subscription(name ="subscribeRelayNetworkStatus",item = RelayServerMetrics )]
    async fn watch_relay_network_status(&self) -> SubscriptionResult;

}

pub struct RelayServerRpcWorker {
    pub relay_server_metrics: Arc<Mutex<Receiver<RelayServerMetrics>>>,
}

impl RelayServerRpcWorker {
    pub fn new(relay_server_metrics: Receiver<RelayServerMetrics>) -> Self {
        Self { 
            relay_server_metrics: Arc::new(Mutex::new(relay_server_metrics))
        }
    }
}

#[async_trait]
impl RelayServerRpcServer for RelayServerRpcWorker {
    async fn watch_relay_network_status(&self, subscription_sink: PendingSubscriptionSink) -> SubscriptionResult {
        let sink = subscription_sink
            .accept()
            .await
            .map_err(|_| anyhow!("failed to accept rpc ws channel"))?;

        while let Some(relay_server_metrics) = self.relay_server_metrics.lock().await.recv().await {
            trace!(target:"rpc","\n watching relay server metrics: {relay_server_metrics:?} \n");

            let subscription_msg = SubscriptionMessage::from_json(&relay_server_metrics)
                .map_err(|_| anyhow!("failed to convert tx update to json"))?;
            sink.send(subscription_msg)
                .await
                .map_err(|_| anyhow!("failed to send msg to rpc ws channel"))?;
        }
        Ok(())
    }
}