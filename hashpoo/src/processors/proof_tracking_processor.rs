use {
    crate::utils,
    base64::{prelude::BASE64_STANDARD, Engine},
    futures::StreamExt,
    ore_api::state::Proof,
    ore_utils::AccountDeserialize,
    solana_account_decoder::UiAccountEncoding,
    solana_client::{nonblocking::pubsub_client::PubsubClient, rpc_config::RpcAccountInfoConfig},
    solana_sdk::{commitment_config::CommitmentConfig, signature::Keypair, signer::Signer},
    std::{sync::Arc, time::Duration},
    tokio::sync::Mutex,
    tracing::{error, info},
};

pub async fn proof_tracking_processor(
    ws_url: String,
    wallet: Arc<Keypair>,
    proof: Arc<Mutex<Proof>>,
    last_challenge: Arc<Mutex<[u8; 32]>>,
) {
    loop {
        info!(target: "server_log", "Establishing rpc websocket connection...");
        let mut ps_client = PubsubClient::new(&ws_url).await;

        while ps_client.is_err() {
            error!(target: "server_log", "Failed to connect to websocket, retrying...");
            ps_client = PubsubClient::new(&ws_url).await;
            tokio::time::sleep(Duration::from_millis(1000)).await;
        }
        info!(target: "server_log", "RPC WS connection established!");

        let app_wallet = wallet.clone();
        if let Ok(ps_client) = ps_client {
            // The `PubsubClient` must be `Arc`ed to share it across threads/tasks.
            let ps_client = Arc::new(ps_client);
            let account_pubkey = utils::mini_pool_proof_pubkey(app_wallet.pubkey());
            let ps_client = Arc::clone(&ps_client); // MI
            let pubsub = ps_client
                .account_subscribe(
                    &account_pubkey,
                    Some(RpcAccountInfoConfig {
                        encoding: Some(UiAccountEncoding::Base64),
                        data_slice: None,
                        commitment: Some(CommitmentConfig::confirmed()),
                        min_context_slot: None,
                    }),
                )
                .await;

            info!(target: "server_log", "Subscribed notification of pool proof updates via websocket");
            if let Ok((mut account_sub_notifications, _account_unsub)) = pubsub {
                // MI: vanilla, by design while let will exit when None received
                while let Some(response) = account_sub_notifications.next().await {
                    let data = response.value.data.decode();
                    if let Some(data_bytes) = data {
                        if let Ok(new_proof) = Proof::try_from_bytes(&data_bytes) {
                            info!(target: "server_log",
                                "Received new proof with challenge: {}",
                                BASE64_STANDARD.encode(new_proof.challenge)
                            );

                            let lock = last_challenge.lock().await;
                            let last_challenge = lock.clone();
                            drop(lock);

                            if last_challenge.eq(&new_proof.challenge) {
                                error!(target: "server_log", "Websocket tried to update proof with existing/old challenge!");
                            } else {
                                let mut app_proof = proof.lock().await;
                                *app_proof = *new_proof;
                                drop(app_proof);
                            }
                        }
                    }
                }

                // // MI: use loop, by design while let will exit when None received
                // loop {
                //     if let Some(response) = account_sub_notifications.next().await {
                //         let data = response.value.data.decode();
                //         if let Some(data_bytes) = data {
                //             if let Ok(new_proof) = Proof::try_from_bytes(&data_bytes) {
                //                 {
                //                     let mut app_proof = proof.lock().await;
                //                     *app_proof = *new_proof;
                //                     drop(app_proof);
                //                 }
                //             }
                //         }
                //     }
                // }
            }
        }
    }
}
