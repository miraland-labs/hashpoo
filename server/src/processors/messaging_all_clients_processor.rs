use {
    crate::{AppState, MessageInternalAllClients},
    axum::extract::ws::Message,
    futures::SinkExt,
    std::sync::Arc,
    tokio::sync::{mpsc::UnboundedReceiver, RwLock},
    tracing::error,
};

pub async fn messaging_all_clients_processor(
    app_shared_state: Arc<RwLock<AppState>>,
    mut all_clients_receiver: UnboundedReceiver<MessageInternalAllClients>,
) {
    loop {
        while let Some(msg) = all_clients_receiver.recv().await {
            {
                let shared_state = app_shared_state.read().await;
                let socks = shared_state.sockets.clone();
                drop(shared_state);
                for (_socket_addr, socket_sender) in socks.iter() {
                    let text = msg.text.clone();
                    let socket = socket_sender.clone();
                    tokio::spawn(async move {
                        if let Ok(_) = socket.socket.lock().await.send(Message::Text(text)).await {
                        } else {
                            error!(target: "server_log", "Failed to send client text");
                        }
                    });
                }
            }
        }
    }
}