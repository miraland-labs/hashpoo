use {
    crate::AppState,
    axum::extract::ws::Message,
    futures::SinkExt,
    std::{sync::Arc, time::Duration},
    tokio::sync::RwLock,
    tracing::error,
};

pub async fn ping_check_processor(shared_state: &Arc<RwLock<AppState>>) {
    loop {
        // send ping to all sockets
        let app_state = shared_state.read().await;
        let socks = app_state.sockets.clone();
        drop(app_state);

        let mut handles = Vec::new();
        for (who, socket) in socks.iter() {
            let who = who.clone();
            let socket = socket.clone();
            handles.push(tokio::spawn(async move {
                if socket.socket.lock().await.send(Message::Ping(vec![1, 2, 3])).await.is_ok() {
                    return None;
                } else {
                    return Some(who.clone());
                }
            }));
        }

        // remove any sockets where ping failed
        for handle in handles {
            match handle.await {
                Ok(Some(who)) => {
                    let mut app_state = shared_state.write().await;
                    app_state.sockets.remove(&who);
                },
                Ok(None) => {},
                Err(_) => {
                    error!(target: "server_log", "Got error sending ping to client.");
                },
            }
        }

        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}
