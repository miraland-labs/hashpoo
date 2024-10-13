use {
    crate::{
        database::Database,
        utils::{self, ORE_TOKEN_DECIMALS},
        ClaimsQueue, InsertClaim, InsertTransaction, CREATE_ATA_DEDUCTION,
    },
    solana_client::{nonblocking::rpc_client::RpcClient, rpc_config::RpcSendTransactionConfig},
    solana_sdk::{
        compute_budget::ComputeBudgetInstruction,
        signature::{Keypair, Signature},
        signer::Signer,
        transaction::Transaction,
    },
    solana_transaction_status::TransactionConfirmationStatus,
    spl_associated_token_account::get_associated_token_address,
    std::{sync::Arc, time::Duration},
    tokio::time::Instant,
    tracing::{error, info},
};

pub async fn claim_processor(
    claims_queue: Arc<ClaimsQueue>,
    rpc_client: Arc<RpcClient>,
    wallet: Arc<Keypair>,
    database: Arc<Database>,
) {
    loop {
        let mut claim = None;
        let reader = claims_queue.queue.read().await;
        let item = reader.iter().next();
        if let Some(item) = item {
            claim = Some((item.0.clone(), item.1.clone()));
        }
        drop(reader);

        if let Some((miner_pubkey, claim_queue_item)) = claim {
            info!(target: "server_log", "Processing claim...");
            let ore_mint = utils::get_ore_mint();
            let receiver_pubkey = claim_queue_item.receiver_pubkey;
            let receiver_token_account = get_associated_token_address(&receiver_pubkey, &ore_mint);

            let prio_fee: u32 = 20_000;

            let mut is_creating_ata = false;
            let mut ixs = Vec::new();
            let prio_fee_ix = ComputeBudgetInstruction::set_compute_unit_price(prio_fee as u64);
            ixs.push(prio_fee_ix);
            if let Ok(response) =
                rpc_client.get_token_account_balance(&receiver_token_account).await
            {
                if let Some(_amount) = response.ui_amount {
                    info!(target: "server_log", "miner has valid token account.");
                } else {
                    info!(target: "server_log", "will create token account for miner");
                    ixs.push(
                        spl_associated_token_account::instruction::create_associated_token_account(
                            &wallet.pubkey(),
                            &receiver_pubkey,
                            &ore_api::consts::MINT_ADDRESS,
                            &spl_token::id(),
                        ),
                    )
                }
            } else {
                info!(target: "server_log", "Adding create ata ix for miner claim");
                is_creating_ata = true;
                ixs.push(
                    spl_associated_token_account::instruction::create_associated_token_account(
                        &wallet.pubkey(),
                        &receiver_pubkey,
                        &ore_api::consts::MINT_ADDRESS,
                        &spl_token::id(),
                    ),
                )
            }

            let amount = claim_queue_item.amount;

            let mut claim_amount = amount;
            // 0.00400000000 ORE
            if is_creating_ata {
                // claim_amount = amount - 400_000_000
                claim_amount = amount - CREATE_ATA_DEDUCTION
            }
            // claim from mini pool (same as operator's solo proof)
            let ix = utils::get_claim_ix(wallet.pubkey(), receiver_token_account, claim_amount);
            ixs.push(ix);

            if let Ok((hash, _slot)) =
                rpc_client.get_latest_blockhash_with_commitment(rpc_client.commitment()).await
            {
                let expired_timer = Instant::now();
                let mut tx = Transaction::new_with_payer(&ixs, Some(&wallet.pubkey()));

                tx.sign(&[&wallet], hash);

                let rpc_config = RpcSendTransactionConfig {
                    preflight_commitment: Some(rpc_client.commitment().commitment),
                    ..RpcSendTransactionConfig::default()
                };

                let signature;
                loop {
                    if let Ok(sig) = rpc_client.send_transaction_with_config(&tx, rpc_config).await
                    {
                        signature = sig;
                        break;
                    } else {
                        error!(target: "server_log", "Failed to send claim transaction. retrying in 2 seconds...");
                        tokio::time::sleep(Duration::from_millis(2000)).await;
                    }
                }

                let result: Result<Signature, String> = loop {
                    if expired_timer.elapsed().as_secs() >= 200 {
                        break Err("Transaction Expired".to_string());
                    }
                    let results = rpc_client.get_signature_statuses(&[signature]).await;
                    if let Ok(response) = results {
                        let statuses = response.value;
                        if let Some(status) = &statuses[0] {
                            if status.confirmation_status()
                                == TransactionConfirmationStatus::Confirmed
                            {
                                if status.err.is_some() {
                                    let e_str = format!("Transaction Failed: {:?}", status.err);
                                    break Err(e_str);
                                }
                                break Ok(signature);
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                };

                match result {
                    Ok(sig) => {
                        let amount_dec = amount as f64 / 10f64.powf(ORE_TOKEN_DECIMALS as f64);
                        info!(target: "server_log", "Miner {} successfully claimed {}.\nSig: {}", miner_pubkey.to_string(), amount_dec, sig.to_string());

                        // TODO: use transacions, or at least put them into one query
                        let miner = database
                            .get_miner_by_pubkey_str(miner_pubkey.to_string())
                            .await
                            .unwrap();
                        let mining_pool = database
                            .get_pool_by_authority_pubkey(wallet.pubkey().to_string())
                            .await
                            .unwrap();
                        while let Err(_) =
                            database.decrease_miner_reward(miner.id, amount as i64).await
                        {
                            error!(target: "server_log", "Failed to decrease miner rewards! Retrying...");
                            tokio::time::sleep(Duration::from_millis(2000)).await;
                        }
                        while let Err(_) = database
                            .update_pool_claimed(wallet.pubkey().to_string(), amount as i64)
                            .await
                        {
                            error!(target: "server_log", "Failed to increase pool claimed amount! Retrying...");
                            tokio::time::sleep(Duration::from_millis(2000)).await;
                        }

                        let itxn = InsertTransaction {
                            transaction_type: "claim".to_string(),
                            signature: sig.to_string(),
                            priority_fee: prio_fee as i32,
                            pool_id: mining_pool.id,
                        };
                        while let Err(_) = database.add_new_transaction(itxn.clone()).await {
                            error!(target: "server_log", "Failed to increase pool claimed amount! Retrying...");
                            tokio::time::sleep(Duration::from_millis(2000)).await;
                        }

                        let txn_id;
                        loop {
                            if let Ok(ntxn) =
                                database.get_transaction_id_by_sig(sig.to_string()).await
                            {
                                txn_id = ntxn.id;
                                break;
                            } else {
                                error!(target: "server_log", "Failed to get tx by sig! Retrying...");
                                tokio::time::sleep(Duration::from_millis(2000)).await;
                            }
                        }

                        let iclaim = InsertClaim {
                            miner_id: miner.id,
                            pool_id: mining_pool.id,
                            transaction_id: txn_id,
                            amount: amount as i64,
                        };
                        while let Err(_) = database.add_new_claim(iclaim).await {
                            error!(target: "server_log", "Failed add new claim to db! Retrying...");
                            tokio::time::sleep(Duration::from_millis(2000)).await;
                        }

                        let mut writer = claims_queue.queue.write().await;
                        writer.remove(&miner_pubkey);
                        drop(writer);

                        info!(target: "server_log", "Claim successfully processed!");
                    },
                    Err(e) => {
                        error!(target: "server_log", "ERROR: {:?}", e);
                    },
                }
            } else {
                error!(target: "server_log", "Failed to confirm transaction, will retry on next iteration.");
            }
        }

        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}
