use crate::telemetry::RelayServerMetrics;
use anyhow::anyhow;
use jsonrpsee::{
    core::{async_trait, RpcResult, SubscriptionResult},
    proc_macros::rpc,
    PendingSubscriptionSink, SubscriptionMessage,
};
use log::{debug, trace};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc::Receiver, Mutex};

#[rpc(server, client)]
pub trait RelayServerRpc {
    /// watch relay network status
    #[subscription(name ="subscribeRelayNetworkStatus",item = RelayServerMetrics )]
    async fn watch_relay_network_status(&self) -> SubscriptionResult;
}

pub struct RelayServerRpcWorker {
    pub relay_server_metrics_broadcast: Arc<broadcast::Sender<RelayServerMetrics>>,
}

impl RelayServerRpcWorker {
    pub fn new(mut relay_server_metrics_channel: Receiver<RelayServerMetrics>) -> Self {
        // Create a broadcast channel that can handle multiple subscribers
        let (broadcast_tx, _) = broadcast::channel(100);
        let broadcast_tx = Arc::new(broadcast_tx);
        
        // Spawn a task to continuously consume from the mpsc channel and forward to broadcast
        let broadcast_tx_clone = broadcast_tx.clone();
        tokio::spawn(async move {
            while let Some(metrics) = relay_server_metrics_channel.recv().await {
                // Send to broadcast channel - if no subscribers, messages are dropped gracefully
                if let Err(_) = broadcast_tx_clone.send(metrics.clone()) {
                    debug!(target: "rpc", "No active subscribers for metrics broadcast");
                }
            }
            debug!(target: "rpc", "Metrics channel receiver closed");
        });
        
        Self {
            relay_server_metrics_broadcast: broadcast_tx,
        }
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

        // Subscribe to the broadcast channel for metrics
        let mut metrics_rx = self.relay_server_metrics_broadcast.subscribe();

        while let Ok(relay_server_metrics) = metrics_rx.recv().await {
            trace!(target:"rpc","\n watching relay server metrics: {relay_server_metrics:?} \n");

            let subscription_msg = SubscriptionMessage::from_json(&relay_server_metrics)
                .map_err(|_| anyhow!("failed to convert tx update to json"))?;
            
            // If sending fails, client disconnected - break the loop gracefully
            if let Err(_) = sink.send(subscription_msg).await {
                debug!(target: "rpc", "Client disconnected from metrics subscription");
                break;
            }
        }
        Ok(())
    }
}
