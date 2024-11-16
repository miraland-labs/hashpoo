#[allow(unused_imports)]
use crate::{
    utils, AppState, ClientMessage, EpochHashes, InternalMessageContribution, LastPong,
    HASHPOWER_CAP, MIN_DIFF, UNIT_HASHPOWER,
};
use {
    axum::extract::ws::Message,
    chrono::Local,
    drillx::Solution,
    futures::SinkExt,
    ore_api::state::Proof,
    solana_sdk::pubkey::Pubkey,
    std::{collections::HashMap, net::SocketAddr, ops::Range, sync::Arc},
    tokio::sync::{mpsc::UnboundedReceiver, Mutex, RwLock},
    tracing::{debug, error, info, warn},
    uuid::Uuid,
};

pub struct ClientBestSolution {
    pub data: (SocketAddr, Solution, Pubkey),
}

pub async fn client_contributions_processor(
    mut receiver_channel: UnboundedReceiver<ClientBestSolution>,
    proof: Arc<Mutex<Proof>>,
    epoch_hashes: Arc<RwLock<EpochHashes>>,
    client_nonce_ranges: Arc<RwLock<HashMap<Pubkey, Range<u64>>>>,
    app_state: Arc<RwLock<AppState>>,
    min_difficulty: u32,
) {
    loop {
        if let Some(client_contribution_message) = receiver_channel.recv().await {
            let (addr, solution, pubkey) = client_contribution_message.data;
            let diff = solution.to_hash().difficulty();
            // if diff >= MIN_DIFF {
            if diff >= min_difficulty {
                let pubkey_str = pubkey.to_string();
                let len = pubkey_str.len();
                let short_pbukey_str =
                    format!("{}...{}", &pubkey_str[0..6], &pubkey_str[len - 4..len]);

                let reader = client_nonce_ranges.read().await;
                let nonce_range: Range<u64> = {
                    if let Some(nr) = reader.get(&pubkey) {
                        nr.clone()
                    } else {
                        error!(target: "server_log", "Client nonce range not set!");
                        return;
                    }
                };
                drop(reader);

                let digest = solution.d; // MI
                let nonce = u64::from_le_bytes(solution.n);

                if !nonce_range.contains(&nonce) {
                    error!(target: "server_log", "❌ Client submitted nonce out of assigned range");
                    continue;
                }

                let reader = app_state.read().await;
                let miner_id;
                if let Some(app_client_socket) = reader.sockets.get(&addr) {
                    miner_id = app_client_socket.miner_id;
                } else {
                    error!(target: "server_log", "Failed to get client socket for addr: {}", addr);
                    continue;
                }
                drop(reader);

                let lock = proof.lock().await;
                let challenge = lock.challenge;
                drop(lock);

                if solution.is_valid(&challenge) {
                    let diff = solution.to_hash().difficulty();
                    let contribution_uuid = Uuid::new_v4();
                    debug!(target: "server_log", "{contribution_uuid} : ");
                    debug!(target: "server_log",
                        "Client {} with pubkey {} found diff: {} at {}",
                        addr.to_string(),
                        // pubkey_str,
                        short_pbukey_str,
                        diff,
                        Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
                    );
                    info!(target: "contribution_log", "{contribution_uuid} : ");
                    info!(target: "contribution_log",
                        "Client {} with pubkey {} found diff: {} at {}",
                        addr.to_string(),
                        // pubkey_str,
                        short_pbukey_str,
                        diff,
                        Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
                    );

                    // calculate rewards, only diff larger than min_difficulty(rather
                    // than MIN_DIFF) qualifies rewards calc.

                    // let hashpower = utils::normalized_hashpower(
                    //     UNIT_HASHPOWER,
                    //     MIN_DIFF,
                    //     diff,
                    //     Some(HASHPOWER_CAP),
                    // );

                    // MI: For hashpoo, no cap limit for hashpower, like natural selection in the wild
                    let hashpower = utils::normalized_hashpower(
                        UNIT_HASHPOWER,
                        MIN_DIFF,
                        diff,
                        None, // No hashpower cap
                    );

                    {
                        let reader = epoch_hashes.read().await;
                        let subs = reader.contributions.clone();
                        drop(reader);

                        if let Some(old_sub) = subs.get(&pubkey) {
                            if diff > old_sub.supplied_diff {
                                let mut epoch_hashes = epoch_hashes.write().await;
                                epoch_hashes.contributions.insert(
                                    pubkey,
                                    InternalMessageContribution {
                                        miner_id,
                                        supplied_digest: digest,
                                        supplied_nonce: nonce,
                                        supplied_diff: diff,
                                        hashpower,
                                    },
                                );
                                if diff > epoch_hashes.best_hash.difficulty {
                                    // info!(target: "server_log", "{} - New best diff:
                                    // {}", contribution_uuid, diff);
                                    info!(target: "contribution_log", "{} - New best diff: {}", contribution_uuid, diff);
                                    epoch_hashes.best_hash.difficulty = diff;
                                    epoch_hashes.best_hash.solution = Some(solution);
                                }
                                drop(epoch_hashes);
                            } else {
                                info!(target: "server_log", "Miner submitted lower diff than a previous contribution, discarding lower diff");
                            }
                        } else {
                            info!(target: "contribution_log", "{} : ", contribution_uuid);
                            info!(target: "contribution_log", "Adding {} contribution diff: {} to epoch_hashes contributions.", pubkey_str, diff);
                            let mut epoch_hashes = epoch_hashes.write().await;
                            epoch_hashes.contributions.insert(
                                pubkey,
                                InternalMessageContribution {
                                    miner_id,
                                    supplied_digest: digest,
                                    supplied_nonce: nonce,
                                    supplied_diff: diff,
                                    hashpower,
                                },
                            );
                            if diff > epoch_hashes.best_hash.difficulty {
                                // info!(target: "server_log", "{} - New best diff: {}",
                                // contribution_uuid, diff);
                                info!(target: "contribution_log", "{} - New best diff: {}", contribution_uuid, diff);
                                epoch_hashes.best_hash.difficulty = diff;
                                epoch_hashes.best_hash.solution = Some(solution);
                            }
                            drop(epoch_hashes);
                            // info!(target: "contribution_log", "{} - Added {}
                            // contribution diff: {} to epoch_hashes contributions.",
                            // contribution_uuid, pubkey_str, diff);
                        }
                    }
                    // tokio::time::sleep(Duration::from_millis(100)).await;
                } else {
                    error!(target: "server_log",
                        "{} returned an invalid solution for latest challenge!",
                        // pubkey
                        short_pbukey_str
                    );

                    let reader = app_state.read().await;
                    if let Some(app_client_socket) = reader.sockets.get(&addr) {
                        let _ = app_client_socket.socket.lock().await.send(Message::Text("Invalid solution. If this keeps happening, please contact support.".to_string())).await;
                    } else {
                        error!(target: "server_log", "Failed to get client socket for addr: {}", addr);
                        continue;
                    }
                    drop(reader);
                }
            } else {
                warn!(target: "server_log", "Diff too low, skipping");
            }
        } else {
            // receiver_channel got None, the stream ended.
            // None is returned when all Sender halves have dropped, indicating that no further
            // values can be sent on the channel.
            warn!(target: "server_log", "All client contribution message senders have been dropped. No more client contribution messages will be received. Exit the loop.");
            break; // exit outer loop
        }
    }
}
