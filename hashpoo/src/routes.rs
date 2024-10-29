use {
    crate::{
        models, rr_database,
        utils::{get_mini_pool_proof, get_ore_mint},
        ChallengeWithDifficulty, MineConfig, WALLET_PUBKEY,
    },
    axum::{
        http::{Response, StatusCode},
        response::IntoResponse,
        Extension, Json,
    },
    rr_database::RrDatabase,
    solana_client::nonblocking::rpc_client::RpcClient,
    spl_associated_token_account::get_associated_token_address,
    std::sync::Arc,
    tracing::error,
};

pub async fn get_challenges(
    Extension(rr_database): Extension<Arc<RrDatabase>>,
    Extension(mine_config): Extension<Arc<MineConfig>>,
) -> Result<Json<Vec<ChallengeWithDifficulty>>, String> {
    if mine_config.stats_enabled {
        let res = rr_database.get_challenges().await;

        match res {
            Ok(challenges) => Ok(Json(challenges)),
            Err(_) => Err("Failed to get challenges for miner".to_string()),
        }
    } else {
        return Err("Stats not enabled for this server.".to_string());
    }
}

pub async fn get_latest_mine_transaction(
    Extension(rr_database): Extension<Arc<RrDatabase>>,
    Extension(mine_config): Extension<Arc<MineConfig>>,
) -> Result<Json<models::Transaction>, String> {
    if mine_config.stats_enabled {
        let res = rr_database.get_latest_mine_transaction().await;

        match res {
            Ok(txn) => Ok(Json(txn)),
            Err(_) => Err("Failed to get latest mine transaction".to_string()),
        }
    } else {
        return Err("Stats not enabled for this server.".to_string());
    }
}

pub async fn get_pool(
    Extension(rr_database): Extension<Arc<RrDatabase>>,
    Extension(mine_config): Extension<Arc<MineConfig>>,
) -> Result<Json<crate::models::Pool>, String> {
    if mine_config.stats_enabled {
        let pubkey = *WALLET_PUBKEY.get().unwrap();
        let res = rr_database.get_pool_by_authority_pubkey(pubkey.to_string()).await;

        match res {
            Ok(pool) => Ok(Json(pool)),
            Err(_) => Err("Failed to get pool data".to_string()),
        }
    } else {
        return Err("Stats not enabled for this server.".to_string());
    }
}

pub async fn get_pool_staked(
    Extension(mine_config): Extension<Arc<MineConfig>>,
    Extension(rpc_client): Extension<Arc<RpcClient>>,
) -> impl IntoResponse {
    if mine_config.stats_enabled {
        let pubkey = *WALLET_PUBKEY.get().unwrap();
        let proof = if let Ok(loaded_proof) = get_mini_pool_proof(&rpc_client, pubkey).await {
            loaded_proof
        } else {
            error!("get_pool_staked: Failed to load mini pool proof.");
            return Err("Stats not enabled for this server.".to_string());
        };

        return Ok(Json(proof.balance));
    } else {
        return Err("Stats not enabled for this server.".to_string());
    }
}

pub async fn get_pool_balance(
    Extension(mine_config): Extension<Arc<MineConfig>>,
    Extension(rpc_client): Extension<Arc<RpcClient>>,
) -> impl IntoResponse {
    if mine_config.stats_enabled {
        let pubkey = *WALLET_PUBKEY.get().unwrap();
        let miner_token_account = get_associated_token_address(&pubkey, &get_ore_mint());
        if let Ok(response) = rpc_client.get_token_account_balance(&miner_token_account).await {
            return Response::builder()
                .status(StatusCode::OK)
                .body(response.ui_amount_string)
                .unwrap();
        } else {
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body("Failed to get token account balance".to_string())
                .unwrap();
        }
    } else {
        return Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .body("Stats not available on this server.".to_string())
            .unwrap();
    }
}
