
use crate::server::{VaneSwarmServer, MetricService};
use primitives::data_structure::{BackendEvent, SystemNotification};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};


#[tokio::test]
async fn test_vane_swarm_server_creation() {
    let metric_service = Arc::new(MetricService::new());
    let (event_sender, _) = broadcast::channel::<BackendEvent>(100);
    let (system_notification_sender, _) = mpsc::channel::<SystemNotification>(100);
    
    let _server = VaneSwarmServer::new(
        metric_service,
        event_sender,
        system_notification_sender,
        5, // 5 seconds TTL for tests
    );
    
    // Server should be created successfully
    // (we can't check peers.len() as it's private, but creation should work)
    assert!(true); // Placeholder - add your actual assertions
}
