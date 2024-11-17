use {
    crate::{
        message::ServerMessageStartMining,
        utils::{get_cutoff, get_cutoff_with_risk},
        AppState, EpochHashes, PAUSED,
    },
    axum::extract::ws::Message,
    base64::{prelude::BASE64_STANDARD, Engine},
    futures::SinkExt,
    ore_api::state::Proof,
    solana_client::nonblocking::rpc_client::RpcClient,
    solana_sdk::pubkey::Pubkey,
    std::{
        collections::{HashMap, HashSet},
        net::SocketAddr,
        ops::Range,
        sync::{atomic::Ordering::Relaxed, Arc},
        time::Duration,
    },
    tokio::sync::{Mutex, RwLock},
    tracing::{error, info},
};

const NONCE_RANGE_SIZE: u64 = 40_000_000;

pub async fn ready_clients_processor(
    rpc_client: Arc<RpcClient>,
    shared_state: Arc<RwLock<AppState>>,
    app_proof: Arc<Mutex<Proof>>,
    epoch_hashes: Arc<RwLock<EpochHashes>>,
    ready_clients: Arc<Mutex<HashSet<SocketAddr>>>,
    app_nonce: Arc<Mutex<u64>>,
    client_nonce_ranges: Arc<RwLock<HashMap<Pubkey, Range<u64>>>>,
    buffer_time: Arc<u64>,
    risk_time: Arc<u64>,
) {
    loop {
        let mut clients = Vec::new();
        {
            let ready_clients_lock = ready_clients.lock().await;
            for ready_client in ready_clients_lock.iter() {
                clients.push(ready_client.clone());
            }
            drop(ready_clients_lock);
        };

        if !PAUSED.load(Relaxed) && clients.len() > 0 {
            let lock = app_proof.lock().await;
            let proof = lock.clone();
            drop(lock);

            let cutoff = if (*risk_time).gt(&0) {
                get_cutoff_with_risk(&rpc_client, proof, *buffer_time, *risk_time).await
            } else {
                get_cutoff(&rpc_client, proof, *buffer_time).await
            };
            let mut should_mine = true;

            // only distribute challenge if 10 seconds or more is left
            // or if there is no best_hash yet
            // MI: client submits contribution followed by an immediate ready-up,
            // which sometimes causes the challenge of the contribution to be redistributed
            // because the server has not yet cutoff. set 10s buffer to avoid such cases.
            let cutoff = if cutoff < 10 {
                let solution = epoch_hashes.read().await.best_hash.solution;
                if solution.is_some() {
                    should_mine = false;
                }
                0
            } else {
                cutoff
            };

            if should_mine {
                let num_clients = clients.len();
                tracing::info!(target: "server_log", "Processing {} ready clients.", num_clients);
                let lock = app_proof.lock().await;
                let proof = lock.clone();
                drop(lock);
                let challenge = proof.challenge;
                info!(target: "server_log", "Mission to clients with challenge: {}", BASE64_STANDARD.encode(challenge));
                info!(target: "contribution_log", "Mission to clients with challenge: {}", BASE64_STANDARD.encode(challenge));
                info!(target: "server_log", "and cutoff in: {}s", cutoff);
                info!(target: "contribution_log", "and cutoff in: {}s", cutoff);
                let shared_state = shared_state.read().await;
                let sockets = shared_state.sockets.clone();
                drop(shared_state);
                for client in clients {
                    let nonce_range = {
                        let mut nonce = app_nonce.lock().await;
                        let start = *nonce;
                        // suppose max hashes possible in 60s for a single client
                        // *nonce += 4_000_000;
                        *nonce += NONCE_RANGE_SIZE;
                        drop(nonce);
                        let nonce_end = start + NONCE_RANGE_SIZE;
                        let end = nonce_end;
                        start..end
                    };

                    let start_mining_message = ServerMessageStartMining::new(
                        challenge,
                        cutoff,
                        nonce_range.start,
                        nonce_range.end,
                    );

                    let client_nonce_ranges = client_nonce_ranges.clone();
                    // let shared_state = shared_state.read().await;
                    // let sockets = shared_state.sockets.clone();
                    // drop(shared_state);
                    if let Some(sender) = sockets.get(&client) {
                        let sender = sender.clone();
                        tokio::spawn(async move {
                            let _ = sender
                                .socket
                                .lock()
                                .await
                                .send(Message::Binary(start_mining_message.to_message_binary()))
                                .await;
                            let _ = client_nonce_ranges
                                .write()
                                .await
                                .insert(sender.pubkey, nonce_range);
                        });
                    } else {
                        error!(target: "server_log", "Mission cannot be delivered to client {} because the client no longer exists in the sockets map.", client);
                    }
                    // remove ready client from list
                    let _ = ready_clients.lock().await.remove(&client);
                }
                tracing::info!(target: "server_log", "Processed {} ready clients.", num_clients);
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
