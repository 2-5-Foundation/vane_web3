use alloc::{format, rc::Rc, string::String};
use core::cell::RefCell;

use anyhow::{anyhow, Error};
use futures::FutureExt;
use futures::StreamExt;
use gloo_timers::future::TimeoutFuture;

use jsonrpsee::core::client::{ClientT, SubscriptionClientT};
use jsonrpsee::core::rpc_params;
use jsonrpsee::wasm_client::{Client, WasmClientBuilder};
use log::{debug, error, info, trace, warn};
use serde_json;

use primitives::data_structure::{
    BackendEvent, NetworkCommand, SignatureType, SwarmMessage, TxStateMachine, VanePayload,
};

#[derive(Clone)]
pub struct P2pEventNotifSubSystem {
    pub sender: Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Sender<BackendEvent>>>,
    pub recv: Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Receiver<BackendEvent>>>,
}

impl P2pEventNotifSubSystem {
    pub fn new(
        sender: Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Sender<BackendEvent>>>,
        recv: Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Receiver<BackendEvent>>>,
    ) -> Self {
        Self { sender, recv }
    }
}

#[derive(Clone)]
pub struct WasmP2pWorker {
    pub live: bool,
    pub backend_url: String,
    pub user_account_id: String,
    pub wasm_p2p_command_recv:
        Rc<RefCell<tokio_with_wasm::alias::sync::mpsc::Receiver<NetworkCommand>>>,
    pub p2p_event_notif_sub_system: Rc<P2pEventNotifSubSystem>,
}

impl WasmP2pWorker {
    pub async fn new(
        live: bool,
        relay_node_url: String,
        user_account_id: String,
        command_recv_channel: tokio_with_wasm::alias::sync::mpsc::Receiver<NetworkCommand>,
        p2p_event_notif_sub_system: Rc<P2pEventNotifSubSystem>,
    ) -> Result<Self, anyhow::Error> {
        let ws_url = if relay_node_url.starts_with("ws://") || relay_node_url.starts_with("wss://")
        {
            relay_node_url.clone()
        } else {
            format!("ws://{}", relay_node_url)
        };

        Ok(Self {
            live,
            backend_url: ws_url,
            user_account_id,
            wasm_p2p_command_recv: Rc::new(RefCell::new(command_recv_channel)),
            p2p_event_notif_sub_system,
        })
    }

    pub async fn start(
        &self,
        sig: SignatureType,
        sender_channel: Rc<
            RefCell<tokio_with_wasm::alias::sync::mpsc::Sender<Result<SwarmMessage, Error>>>,
        >,
    ) -> Result<(), Error> {
        let user_account_id = self.user_account_id.clone();
        let p2p_event_notif = self.p2p_event_notif_sub_system.clone();
        let sender = sender_channel.clone();
        let p2p_command_recv = self.wasm_p2p_command_recv.clone();
        let backend_url = self.backend_url.clone();

        wasm_bindgen_futures::spawn_local(async move {
            loop {
                // Try to connect
                let jsonrpc_client = match WasmClientBuilder::default().build(&backend_url).await {
                    Ok(client) => {
                        info!(target: "p2p", "Connected to backend at: {}", backend_url);
                        client
                    }
                    Err(e) => {
                        error!(target: "p2p", "Failed to create JSON-RPC client: {}. Retrying in 1s", e);
                        TimeoutFuture::new(1000).await;
                        continue;
                    }
                };

                // Try to subscribe
                let mut subscription: jsonrpsee::core::client::Subscription<BackendEvent> =
                    match jsonrpc_client
                        .subscribe(
                            "subscribeToEvents",
                            rpc_params![user_account_id.clone(), sig.clone()],
                            "unsubscribe_subscribeToEvents",
                        )
                        .await
                    {
                        Ok(sub) => {
                            info!(target: "p2p", "Subscribed to backend events for address: {}", user_account_id);
                            sub
                        }
                        Err(e) => {
                            error!(target: "p2p", "Failed to subscribe to events: {}. Retrying in 1s", e);
                            TimeoutFuture::new(1000).await;
                            continue;
                        }
                    };

                let client = Rc::new(jsonrpc_client);
                let client_clone = client.clone();

                // Per-connection loop
                loop {
                    let mut p2p_command_recv = p2p_command_recv.borrow_mut();

                    futures::select! {
                        event_result = subscription.next().fuse() => {
                            match event_result {
                                Some(Ok(event)) => {

                                    if let Err(e) = p2p_event_notif.sender.borrow_mut().send(event.clone()).await {
                                        error!(target: "p2p", "Failed to send backend event: {}", e);
                                    }

                                    match event {
                                        BackendEvent::SenderRequestHandled { address: _, data } => {
                                            if let Ok(tx_state) = serde_json::from_slice::<TxStateMachine>(&data) {
                                                if tx_state.receiver_address == user_account_id {
                                                    let req_msg = SwarmMessage::WasmRequest { data: tx_state };
                                                    if let Err(e) = sender.borrow_mut().send(Ok(req_msg)).await {
                                                        error!(target: "p2p", "Failed to send request message: {}", e);
                                                    }
                                                }
                                            }
                                        }

                                        BackendEvent::ReceiverResponseHandled { address: _, data } => {
                                            if let Ok(tx_state) = serde_json::from_slice::<TxStateMachine>(&data) {
                                                if tx_state.sender_address == user_account_id {
                                                    let resp_msg = SwarmMessage::WasmResponse { data: tx_state };
                                                    if let Err(e) = sender.borrow_mut().send(Ok(resp_msg)).await {
                                                        error!(target: "p2p", "Failed to send response message: {}", e);
                                                    }
                                                }
                                            }
                                        }
                                        BackendEvent::PendingTransactionsFetched { address, transactions } => {
                                            let pending_txs_msg = SwarmMessage::PendingTransactionsFetched { address,transactions };

                                            if let Err(e) = sender.borrow_mut().send(Ok(pending_txs_msg)).await {
                                                error!(target: "p2p", "Failed to send pending transactions message: {}", e);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                Some(Err(e)) => {
                                    error!(target: "p2p", "Error receiving event from subscription: {}", e);
                                    break;
                                }
                                None => {
                                    warn!(target: "p2p", "Event subscription ended");
                                    break;
                                }
                            }
                        },
                        cmd = p2p_command_recv.recv().fuse() => {
                            match cmd {
                                Some(NetworkCommand::WasmSendRequest { request }) => {
                                    let client = client_clone.clone();
                                    let sig = request.extra_data.clone();
                                    let data = request.data.clone();
                                    let address = data.sender_address.clone();
                                    let data = match serde_json::to_vec(&data) {
                                        Ok(data) => data,
                                        Err(e) => {
                                            error!(target: "p2p", "Failed to serialize request: {}", e);
                                            continue;
                                        }
                                    };

                                    wasm_bindgen_futures::spawn_local(async move {
                                        match client
                                            .request::<(), _>(
                                                "handleSenderRequest",
                                                rpc_params![sig, address.clone(), data],
                                            )
                                            .await
                                        {
                                            Ok(_) => {
                                                info!(target: "p2p", "Successfully sent sender request for address: {}", address);
                                            }
                                            Err(e) => {
                                                error!(target: "p2p", "Failed to send sender request: {}", e);
                                            }
                                        }
                                    });
                                }
                                Some(NetworkCommand::WasmSendResponse { response }) => {
                                    let client = client_clone.clone();

                                    match response {
                                        Ok(vane_payload) => {
                                            let tx_state = vane_payload.data.clone();
                                            let sig = vane_payload.extra_data.clone();
                                            let address = tx_state.receiver_address.clone();
                                            let data = match serde_json::to_vec(&tx_state) {
                                                Ok(data) => data,
                                                Err(e) => {
                                                    error!(target: "p2p", "Failed to serialize response: {}", e);
                                                    continue;
                                                }
                                            };

                                            wasm_bindgen_futures::spawn_local(async move {
                                                match client
                                                    .request::<(), _>(
                                                        "handleReceiverResponse",
                                                        rpc_params![sig, address.clone(), data],
                                                    )
                                                    .await
                                                {
                                                    Ok(_) => {
                                                        info!(target: "p2p", "Successfully sent receiver response for address: {}", address);
                                                    }
                                                    Err(e) => {
                                                        error!(target: "p2p", "Failed to send receiver response: {}", e);
                                                    }
                                                }
                                            });
                                        }
                                        Err(e) => {
                                            error!(target: "p2p", "Response error: {}", e);
                                        }
                                    }
                                }
                                Some(NetworkCommand::FetchPendingTransactions { sig, account_id }) => {
                                    let client = client_clone.clone();
                                    let account_id_clone = account_id.clone();

                                    wasm_bindgen_futures::spawn_local(async move {

                                        match client
                                            .request::<(), _>("fetchPendingTransactions", rpc_params![sig, account_id_clone])
                                            .await
                                        {
                                            Ok(_) => {
                                                info!(target: "p2p", "Successfully fetched pending transactions");
                                            }
                                            Err(e) => {
                                                error!(target: "p2p", "Failed to fetch pending transactions: {}", e);
                                            }
                                        }
                                    });
                                }
                                Some(NetworkCommand::ConfirmTransaction { sig, account_id, data }) => {
                                    let client = client_clone.clone();
                                    let account_id_clone = account_id.clone();
                                    let data_bytes = serde_json::to_vec(&data).unwrap_or_default();

                                    wasm_bindgen_futures::spawn_local(async move {
                                        match client
                                            .request::<(), _>(
                                                "handleSenderConfirmation",
                                                rpc_params![sig, account_id_clone, data_bytes],
                                            )
                                            .await
                                        {
                                            Ok(_) => {
                                                info!(target: "p2p", "Successfully sent sender confirmation");
                                            }
                                            Err(e) => {
                                                error!(target: "p2p", "Failed to send sender confirmation: {}", e);
                                            }
                                        }
                                    });
                                }
                                Some(NetworkCommand::TxSubmissionUpdate { sig, account_id, data }) => {
                                    let client = client_clone.clone();
                                    let account_id_clone = account_id.clone();
                                    let data_bytes = serde_json::to_vec(&data).unwrap_or_default();

                                    wasm_bindgen_futures::spawn_local(async move {
                                        match client
                                            .request::<(), _>(
                                                "handleTxSubmissionUpdates",
                                                rpc_params![sig, account_id_clone, data_bytes],
                                            )
                                            .await
                                        {
                                            Ok(_) => {
                                                info!(target: "p2p", "Successfully tx submission update");
                                            }
                                            Err(e) => {
                                                error!(target: "p2p", "Failed tx submission update: {}", e);
                                            }
                                        }
                                    });
                                }
                                Some(NetworkCommand::RevertTransaction { sig, account_id, data }) => {
                                    let client = client_clone.clone();
                                    let account_id_clone = account_id.clone();
                                    let data_bytes = serde_json::to_vec(&data).unwrap_or_default();

                                    wasm_bindgen_futures::spawn_local(async move {
                                        match client
                                            .request::<(), _>(
                                                "handleSenderRevertation",
                                                rpc_params![sig, account_id_clone, data_bytes],
                                            )
                                            .await
                                        {
                                            Ok(_) => {
                                                info!(target: "p2p", "Successfully updating sender revertation");
                                            }
                                            Err(e) => {
                                                error!(target: "p2p", "Failed to sender revertation: {}", e);
                                            }
                                        }
                                    });
                                }
                                _ => {
                                    debug!(target: "p2p", "Unhandled network command");
                                }
                            }
                        }
                    }
                }

                // If we reach here, connection/subscription died; retry
                warn!(target: "p2p", "Disconnected from backend, retrying in 1s");
                TimeoutFuture::new(1000).await;
            }
        });

        Ok(())
    }
}

#[derive(Clone)]
pub struct P2pNetworkService {
    pub p2p_command_tx: Rc<tokio_with_wasm::alias::sync::mpsc::Sender<NetworkCommand>>,
}

impl P2pNetworkService {
    pub fn new(
        p2p_command_tx: Rc<tokio_with_wasm::alias::sync::mpsc::Sender<NetworkCommand>>,
    ) -> Result<Self, Error> {
        Ok(Self { p2p_command_tx })
    }

    pub async fn wasm_send_request(
        &mut self,
        request: Rc<RefCell<VanePayload<TxStateMachine, SignatureType>>>,
    ) -> Result<(), Error> {
        let req = request.borrow().clone();

        let req_command = NetworkCommand::WasmSendRequest { request: req };

        self.p2p_command_tx
            .send(req_command)
            .await
            .map_err(|err| anyhow!("failed to send req command; {err}"))?;
        trace!(target: "p2p", "Sending request command to backend");

        Ok(())
    }

    pub async fn wasm_send_response(
        &mut self,
        response: Rc<RefCell<VanePayload<TxStateMachine, SignatureType>>>,
    ) -> Result<(), anyhow::Error> {
        let txn_state = response.borrow().clone();

        let resp_command = NetworkCommand::WasmSendResponse {
            response: Ok(txn_state),
        };
        self.p2p_command_tx.send(resp_command).await?;
        trace!(target: "p2p", "Sending response command to backend");

        Ok(())
    }
}
