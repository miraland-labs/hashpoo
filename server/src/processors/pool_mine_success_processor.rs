use {
    crate::{
        database::{Database, PoweredByDbms},
        message::ServerMessagePoolSubmissionResult,
        utils::ORE_TOKEN_DECIMALS,
        AppState, ClientVersion, InsertContribution, InsertEarning, InternalMessageContribution,
        MessageInternalMineSuccess, MineConfig, UpdateReward, WalletExtension, POWERED_BY_DBMS,
    },
    axum::extract::ws::Message,
    base64::{prelude::BASE64_STANDARD, Engine},
    futures::SinkExt,
    solana_client::nonblocking::rpc_client::RpcClient,
    solana_sdk::{native_token::LAMPORTS_PER_SOL, signer::Signer},
    std::{ops::Div, str::FromStr, sync::Arc, time::Duration},
    tokio::{
        sync::{mpsc::UnboundedReceiver, RwLock},
        time::Instant,
    },
    tracing::{error, info},
};

pub async fn pool_mine_success_processor(
    app_rpc_client: Arc<RpcClient>,
    app_mine_config: Arc<MineConfig>,
    app_shared_state: Arc<RwLock<AppState>>,
    app_database: Arc<Database>,
    app_wallet: Arc<WalletExtension>,
    mut mine_success_receiver: UnboundedReceiver<MessageInternalMineSuccess>,
) {
    let database = app_database;
    let mine_config = app_mine_config;
    let powered_by_dbms = POWERED_BY_DBMS.get_or_init(|| {
        let key = "POWERED_BY_DBMS";
        match std::env::var(key) {
            Ok(val) => {
                PoweredByDbms::from_str(&val).expect("POWERED_BY_DBMS must be set correctly.")
            },
            Err(_) => PoweredByDbms::Unavailable,
        }
    });
    loop {
        let mut sol_balance_checking = 0_u64;
        while let Some(msg) = mine_success_receiver.recv().await {
            let decimals = 10f64.powf(ORE_TOKEN_DECIMALS as f64);
            let top_stake = if let Some(config) = msg.ore_config {
                (config.top_balance as f64).div(decimals)
            } else {
                1.0f64
            };
            let id = uuid::Uuid::new_v4();
            let c = BASE64_STANDARD.encode(msg.challenge);
            info!(target: "server_log", "{} - Processing internal mine success for challenge: {}", id, c);
            let instant = Instant::now();
            info!(target: "server_log", "{} - Getting sockets.", id);
            let shared_state = app_shared_state.read().await;
            let len = shared_state.sockets.len();
            let socks = shared_state.sockets.clone();
            drop(shared_state);
            info!(target: "server_log", "{} - Got sockets in {}ms.", id, instant.elapsed().as_millis());

            if powered_by_dbms == &PoweredByDbms::Postgres
                || powered_by_dbms == &PoweredByDbms::Sqlite
            {
                let mut i_earnings = Vec::new();
                let mut i_rewards = Vec::new();
                let mut i_contributions = Vec::new();

                let instant = Instant::now();
                info!(target: "server_log", "{} - Processing contribution results for challenge: {}.", id, c);
                let total_rewards = msg.rewards - msg.commissions;
                for (miner_pubkey, msg_contribution) in msg.contributions.iter() {
                    let hashpower_percent = (msg_contribution.hashpower as u128)
                        .saturating_mul(1_000_000)
                        .saturating_div(msg.total_hashpower as u128);

                    // let decimals = 10f64.powf(ORE_TOKEN_DECIMALS as f64);
                    let earned_rewards = hashpower_percent
                        .saturating_mul(total_rewards as u128)
                        .saturating_div(1_000_000) as i64;

                    let new_earning = InsertEarning {
                        miner_id: msg_contribution.miner_id,
                        pool_id: mine_config.pool_id,
                        challenge_id: msg.challenge_id,
                        amount: earned_rewards,
                    };

                    let new_contribution = InsertContribution {
                        miner_id: msg_contribution.miner_id,
                        challenge_id: msg.challenge_id,
                        nonce: msg_contribution.supplied_nonce,
                        digest: msg_contribution.supplied_digest.to_vec(),
                        difficulty: msg_contribution.supplied_diff as i16,
                    };

                    let new_reward = UpdateReward {
                        miner_id: msg_contribution.miner_id,
                        balance: earned_rewards,
                    };

                    i_earnings.push(new_earning);
                    i_rewards.push(new_reward);
                    i_contributions.push(new_contribution);

                    let earned_rewards_dec = (earned_rewards as f64).div(decimals);
                    let pool_rewards_dec = (msg.rewards as f64).div(decimals);

                    let percentage = if pool_rewards_dec != 0.0 {
                        (earned_rewards_dec / pool_rewards_dec) * 100.0
                    } else {
                        0.0 // Handle the case where pool_rewards_dec is 0 to avoid division by
                            // zero
                    };

                    // let top_stake = if let Some(config) = msg.ore_config {
                    //     (config.top_balance as f64).div(decimals)
                    // } else {
                    //     1.0f64
                    // };

                    for (_addr, client_connection) in socks.iter() {
                        if client_connection.pubkey.eq(&miner_pubkey) {
                            let socket_sender = client_connection.socket.clone();
                            match client_connection.client_version {
                                ClientVersion::V0 => {
                                    let message = format!(
                                        "Pool Submitted Difficulty: {}\nPool Earned:  {:.11} ORE\nPool Balance: {:.11} ORE\nTop Stake:    {:.11} ORE\nPool Multiplier: {:.2}x\n----------------------\nActive Miners: {}\n----------------------\nMiner Submitted Difficulty: {}\nMiner Earned: {:.11} ORE\n{:.2}% of total pool reward",
                                        msg.difficulty,
                                        pool_rewards_dec,
                                        msg.total_balance,
                                        top_stake,
                                        msg.multiplier,
                                        len,
                                        msg_contribution.supplied_diff,
                                        earned_rewards_dec,
                                        percentage
                                    );
                                    tokio::spawn(async move {
                                        if let Ok(_) = socket_sender
                                            .lock()
                                            .await
                                            .send(Message::Text(message))
                                            .await
                                        {
                                        } else {
                                            error!(target: "server_log", "Failed to send client text");
                                        }
                                    });
                                },
                                ClientVersion::V1 => {
                                    let server_message = ServerMessagePoolSubmissionResult::new(
                                        msg.difficulty,
                                        msg.total_balance,
                                        pool_rewards_dec,
                                        top_stake,
                                        msg.multiplier,
                                        len as u32,
                                        msg.challenge,
                                        msg.best_nonce,
                                        msg_contribution.supplied_diff as u32,
                                        earned_rewards_dec,
                                        percentage,
                                    );
                                    tokio::spawn(async move {
                                        if let Ok(_) = socket_sender
                                            .lock()
                                            .await
                                            .send(Message::Binary(
                                                server_message.to_message_binary(),
                                            ))
                                            .await
                                        {
                                        } else {
                                            error!(target: "server_log", "Failed to send client pool submission result binary message");
                                        }
                                    });
                                },
                            }
                        }
                    }
                }

                info!(target: "server_log", "{} - Finished processing contribution results in {}ms for challenge: {}.", id, instant.elapsed().as_millis(), c);

                let instant = Instant::now();
                info!(target: "server_log", "{} - Adding earnings", id);
                let batch_size = 200;
                if i_earnings.len() > 0 {
                    for batch in i_earnings.chunks(batch_size) {
                        while let Err(_) = database.add_new_earnings_batch(batch.to_vec()).await {
                            tracing::error!(target: "server_log", "{} - Failed to add new earnings batch to db. Retrying...", id);
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                    info!(target: "server_log", "{} - Successfully added earnings batch", id);
                }
                info!(target: "server_log", "{} - Added earnings in {}ms", id, instant.elapsed().as_millis());

                tokio::time::sleep(Duration::from_millis(500)).await;

                let instant = Instant::now();
                info!(target: "server_log", "{} - Updating rewards", id);
                if i_rewards.len() > 0 {
                    let mut batch_num = 1;
                    for batch in i_rewards.chunks(batch_size) {
                        let instant = Instant::now();
                        info!(target: "server_log", "{} - Updating reward batch {}", id, batch_num);
                        while let Err(_) = database.update_rewards(batch.to_vec()).await {
                            error!(target: "server_log", "{} - Failed to update rewards in db. Retrying...", id);
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                        info!(target: "server_log", "{} - Updated reward batch {} in {}ms", id, batch_num, instant.elapsed().as_millis());
                        batch_num += 1;
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                    info!(target: "server_log", "{} - Successfully updated rewards", id);
                }
                info!(target: "server_log", "{} - Updated rewards in {}ms", id, instant.elapsed().as_millis());

                tokio::time::sleep(Duration::from_millis(500)).await;

                let instant = Instant::now();
                info!(target: "server_log", "{} - Adding contributions", id);
                if i_contributions.len() > 0 {
                    for batch in i_contributions.chunks(batch_size) {
                        info!(target: "server_log", "{} - Contributions batch size: {}", id, i_contributions.len());
                        while let Err(_) =
                            database.add_new_contributions_batch(batch.to_vec()).await
                        {
                            error!(target: "server_log", "{} - Failed to add new contributions batch. Retrying...", id);
                            tokio::time::sleep(Duration::from_millis(500)).await;
                        }
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }

                    info!(target: "server_log", "{} - Successfully added contributions batch", id);
                }
                info!(target: "server_log", "{} - Added contributions in {}ms", id, instant.elapsed().as_millis());

                tokio::time::sleep(Duration::from_millis(500)).await;

                let instant = Instant::now();
                info!(target: "server_log", "{} - Updating pool rewards", id);
                while let Err(_) = database
                    .update_pool_rewards(app_wallet.miner_wallet.pubkey().to_string(), msg.rewards)
                    .await
                {
                    error!(target: "server_log", "{} - Failed to update pool rewards! Retrying...", id);
                    tokio::time::sleep(Duration::from_millis(1000)).await;
                }
                info!(target: "server_log", "{} - Updated pool rewards in {}ms", id, instant.elapsed().as_millis());

                tokio::time::sleep(Duration::from_millis(200)).await;

                let instant = Instant::now();
                info!(target: "server_log", "{} - Updating challenge rewards", id);
                if let Ok(s) = database.get_contribution_id_with_nonce(msg.best_nonce).await {
                    if let Err(_) = database
                        .update_challenge_rewards(msg.challenge.to_vec(), s, msg.rewards)
                        .await
                    {
                        error!(target: "server_log", "{} - Failed to update challenge rewards! Skipping! Devs check!", id);
                        let err_str = format!("{} - Challenge UPDATE FAILED - Challenge: {:?}\nContribution ID: {}\nRewards: {}\n", id, msg.challenge.to_vec(), s, msg.rewards);
                        error!(target: "server_log", err_str);
                    }
                    info!(target: "server_log", "{} - Updated challenge rewards in {}ms", id, instant.elapsed().as_millis());
                } else {
                    error!(target: "server_log", "{} - Failed to get contribution id with nonce: {} for challenge_id: {}", id, msg.best_nonce, msg.challenge_id);
                    error!(target: "server_log", "{} - Failed update challenge rewards!", id);
                    let mut found_best_nonce = false;
                    for contribution in i_contributions {
                        if contribution.nonce == msg.best_nonce {
                            found_best_nonce = true;
                            break;
                        }
                    }

                    if found_best_nonce {
                        info!(target: "server_log", "{} - Found best nonce in i_contributions", id);
                    } else {
                        info!(target: "server_log", "{} - Failed to find best nonce in i_contributions", id);
                    }
                }
                info!(target: "server_log", "{} - Finished processing internal mine success for challenge: {}", id, c);
            } else {
                // let decimals = 10f64.powf(ORE_TOKEN_DECIMALS as f64);
                let pool_rewards_dec = (msg.rewards as f64).div(decimals);
                let shared_state = app_shared_state.read().await;
                let len = shared_state.sockets.len();
                let socks = shared_state.sockets.clone();
                drop(shared_state);

                for (_addr, client_connection) in socks.iter() {
                    let socket_sender = client_connection.socket.clone();
                    let pubkey = client_connection.pubkey;

                    if let Some(InternalMessageContribution {
                        miner_id: _,
                        supplied_diff,
                        supplied_digest: _,
                        supplied_nonce: _,
                        hashpower: pubkey_hashpower,
                    }) = msg.contributions.get(&pubkey)
                    {
                        let hashpower_percent = (*pubkey_hashpower as u128)
                            .saturating_mul(1_000_000)
                            .saturating_div(msg.total_hashpower as u128);

                        let earned_rewards = hashpower_percent
                            .saturating_mul(msg.rewards as u128)
                            .saturating_div(1_000_000)
                            as u64;

                        let earned_rewards_dec = (earned_rewards as f64).div(decimals);

                        let percentage = if pool_rewards_dec != 0.0 {
                            (earned_rewards_dec / pool_rewards_dec) * 100.0
                        } else {
                            0.0 // Handle the case where pool_rewards_dec is 0 to avoid division
                                // by zero
                        };

                        match client_connection.client_version {
                            ClientVersion::V0 => {
                                let message = format!(
                                        "Pool Submitted Difficulty: {}\nPool Earned:  {:.11} ORE\nPool Balance: {:.11} ORE\nTop Stake:    {:.11} ORE\nPool Multiplier: {:.2}x\n----------------------\nActive Miners: {}\n----------------------\nMiner Submitted Difficulty: {}\nMiner Earned: {:.11} ORE\n{:.2}% of total pool reward",
                                        msg.difficulty,
                                        pool_rewards_dec,
                                        msg.total_balance,
                                        top_stake,
                                        msg.multiplier,
                                        len,
                                        supplied_diff,
                                        earned_rewards_dec,
                                        percentage
                                    );
                                tokio::spawn(async move {
                                    if let Ok(_) = socket_sender
                                        .lock()
                                        .await
                                        .send(Message::Text(message))
                                        .await
                                    {
                                    } else {
                                        error!(target: "server_log", "Failed to send client text");
                                    }
                                });
                            },
                            ClientVersion::V1 => {
                                let server_message = ServerMessagePoolSubmissionResult::new(
                                    msg.difficulty,
                                    msg.total_balance,
                                    pool_rewards_dec,
                                    top_stake,
                                    msg.multiplier,
                                    len as u32,
                                    msg.challenge,
                                    msg.best_nonce,
                                    *supplied_diff as u32,
                                    earned_rewards_dec,
                                    percentage,
                                );
                                tokio::spawn(async move {
                                    if let Ok(_) = socket_sender
                                        .lock()
                                        .await
                                        .send(Message::Binary(server_message.to_message_binary()))
                                        .await
                                    {
                                    } else {
                                        error!(target: "server_log", "Failed to send client pool submission result binary message");
                                    }
                                });
                            },
                        }
                    }
                }
            }

            if sol_balance_checking % 10 == 0 {
                if let Ok(balance) =
                    app_rpc_client.get_balance(&app_wallet.miner_wallet.pubkey()).await
                {
                    info!(target: "server_log",
                        "Sol Balance(of miner wallet): {:.9}",
                        balance as f64 / LAMPORTS_PER_SOL as f64
                    );
                } else {
                    error!(target: "server_log", "Failed to load balance");
                }
            }
            sol_balance_checking += 1;
        }
    }
}
