#[allow(unused_imports)]
use crate::{
    utils, AppState, ClientMessage, EpochHashes, InternalMessageContribution, LastPong,
    HASHPOWER_CAP, MIN_DIFF, UNIT_HASHPOWER,
};
use {
    super::client_contributions_processor::{client_contributions_processor, ClientBestSolution},
    ore_api::state::Proof,
    solana_sdk::pubkey::Pubkey,
    std::{
        collections::{HashMap, HashSet},
        net::SocketAddr,
        ops::Range,
        sync::Arc,
    },
    tokio::{
        sync::{mpsc::UnboundedReceiver, Mutex, RwLock},
        time::Instant,
    },
    tracing::{info, warn},
};

pub async fn client_message_processor(
    app_state: Arc<RwLock<AppState>>,
    mut receiver_channel: UnboundedReceiver<ClientMessage>,
    epoch_hashes: Arc<RwLock<EpochHashes>>,
    ready_clients: Arc<Mutex<HashSet<SocketAddr>>>,
    proof: Arc<Mutex<Proof>>,
    client_nonce_ranges: Arc<RwLock<HashMap<Pubkey, Range<u64>>>>,
    app_pongs: Arc<RwLock<LastPong>>,
    min_difficulty: u32,
) {
    let (s, r) = tokio::sync::mpsc::unbounded_channel::<ClientBestSolution>();

    let app_proof = proof.clone();
    let app_epoch_hashes = epoch_hashes.clone();
    let app_client_nonce_ranges = client_nonce_ranges.clone();
    let app_app_state = app_state.clone();
    tokio::spawn(async move {
        client_contributions_processor(
            r,
            app_proof,
            app_epoch_hashes,
            app_client_nonce_ranges,
            app_app_state,
            min_difficulty,
        )
        .await;
    });

    loop {
        if let Some(client_message) = receiver_channel.recv().await {
            match client_message {
                ClientMessage::Pong(addr) => {
                    let mut writer = app_pongs.write().await;
                    writer.pongs.insert(addr, Instant::now());
                    drop(writer);
                },
                ClientMessage::Ready(addr) => {
                    let ready_clients = ready_clients.clone();
                    let mut lock = ready_clients.lock().await;
                    lock.insert(addr);
                    drop(lock);
                },
                ClientMessage::Mining(addr) => {
                    info!(target: "server_log", "Client {} has started mining!", addr.to_string());
                },
                ClientMessage::BestSolution(addr, solution, pubkey) => {
                    let _ = s.send(ClientBestSolution { data: (addr, solution, pubkey) });
                },
            }
        } else {
            // receiver_channel got None, the stream ended.
            // None is returned when all Sender halves have dropped, indicating that no further
            // values can be sent on the channel.
            warn!(target: "server_log", "All client message senders have been dropped. No more client messages will be received. Exit the loop.");
            break; // exit outer loop
        }
    }
}
