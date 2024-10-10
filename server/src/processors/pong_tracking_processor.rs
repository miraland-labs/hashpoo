use {
    crate::{AppState, LastPong},
    std::{sync::Arc, time::Duration},
    tokio::sync::RwLock,
    tracing::{error, info},
};

pub async fn pong_tracking_processor(
    app_pongs: Arc<RwLock<LastPong>>,
    app_state: Arc<RwLock<AppState>>,
) {
    loop {
        let reader = app_pongs.read().await;
        let pongs = reader.pongs.clone();
        drop(reader);

        info!(target: "server_log", "Pongs length: {}", pongs.len());

        for pong in pongs.iter() {
            if pong.1.elapsed().as_secs() > 90 {
                error!(target: "server_log", "Failed to get pong within 90s from client on socket: {}", pong.0);
                let mut writer = app_state.write().await;
                writer.sockets.remove(pong.0);
                drop(writer);

                let mut writer = app_pongs.write().await;
                writer.pongs.remove(pong.0);
                drop(writer)
            }
        }

        tokio::time::sleep(Duration::from_secs(45)).await;
    }
}
