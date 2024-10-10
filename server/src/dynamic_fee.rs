use {
    ore_api::consts::BUS_ADDRESSES,
    reqwest::Client,
    serde_json::{json, Value},
    solana_client::{nonblocking::rpc_client::RpcClient, rpc_response::RpcPrioritizationFee},
    solana_sdk::pubkey::Pubkey,
    std::{collections::HashMap, str::FromStr},
    tracing::warn,
    url::Url,
};

pub const DEFAULT_PRIORITY_FEE: u64 = 10_000;

enum FeeStrategy {
    Helius,
    Triton,
    LOCAL,
    Alchemy,
    Quiknode,
}

pub async fn dynamic_fee(
    rpc_client: &RpcClient,
    dynamic_fee_url: Option<String>,
    priority_fee_cap: Option<u64>,
) -> Result<u64, String> {
    // Get rpc url for fee estimate
    let rpc_url = if let Some(dynamic_fee_url) = dynamic_fee_url {
        dynamic_fee_url
    } else {
        rpc_client.url()
    };

    // Select fee estiamte strategy
    let host = Url::parse(&rpc_url).unwrap().host_str().unwrap().to_string();
    let strategy = if host.contains("helius-rpc.com") {
        FeeStrategy::Helius
    } else if host.contains("alchemy.com") {
        FeeStrategy::Alchemy
    } else if host.contains("quiknode.pro") {
        FeeStrategy::Quiknode
    } else if host.contains("rpcpool.com") {
        FeeStrategy::Triton
    } else {
        FeeStrategy::LOCAL
    };

    // Build fee estimate request
    let client = Client::new();
    let ore_addresses: Vec<String> = std::iter::once(ore_api::ID.to_string())
        .chain(BUS_ADDRESSES.iter().map(|pubkey| pubkey.to_string()))
        .collect();
    let body = match strategy {
        FeeStrategy::Helius => Some(json!({
            "jsonrpc": "2.0",
            "id": "priority-fee-estimate",
            "method": "getPriorityFeeEstimate",
            "params": [{
                "accountKeys": ore_addresses,
                "options": {
                    "recommended": true
                }
            }]
        })),
        FeeStrategy::Alchemy => Some(json!({
            "jsonrpc": "2.0",
            "id": "priority-fee-estimate",
            "method": "getRecentPrioritizationFees",
            "params": [
                ore_addresses
            ]
        })),
        FeeStrategy::Quiknode => Some(json!({
            "jsonrpc": "2.0",
            "id": "1",
            "method": "qn_estimatePriorityFees",
            "params": {
                "account": "oreV2ZymfyeXgNgBdqMkumTqqAprVqgBWQfoYkrtKWQ",
                "last_n_blocks": 100
            }
        })),
        FeeStrategy::Triton => Some(json!({
            "jsonrpc": "2.0",
            "id": "priority-fee-estimate",
            "method": "getRecentPrioritizationFees",
            "params": [
                ore_addresses,
                {
                    "percentile": 5000,
                }
            ]
        })),
        FeeStrategy::LOCAL => None,
    };

    // Send rpc request
    let response = if let Some(body) = body {
        // MI, Send request in two steps
        // split json from send
        // 1) handle response
        let Ok(resp) = client.post(rpc_url).json(&body).send().await else {
            // eprintln!("didn't get dynamic fee estimate, use default instead.");
            warn!(target: "server_log", "didn't get dynamic fee estimate, use default instead.");
            return Ok(DEFAULT_PRIORITY_FEE);
        };

        // 2) handle json
        let Ok(response) = resp.json::<Value>().await else {
            // eprintln!("didn't get json data from fee estimate response, use default instead.");
            warn!(target: "server_log", "didn't get json data from fee estimate response, use default instead.");
            return Ok(DEFAULT_PRIORITY_FEE);
        };

        response
    } else {
        Value::Null
    };

    // Parse response
    let calculated_fee = match strategy {
        FeeStrategy::Helius => response["result"]["priorityFeeEstimate"]
            .as_f64()
            .map(|fee| fee as u64)
            .ok_or_else(|| format!("Failed to parse priority fee response: {:?}", response)),
        FeeStrategy::Quiknode => response["result"]["per_compute_unit"]["medium"]
            .as_f64()
            .map(|fee| fee as u64)
            .ok_or_else(|| {
                format!(
                    "Please enable the Solana Priority Fee API add-on in your QuickNode account."
                )
            }),
        FeeStrategy::Alchemy => response["result"]
            .as_array()
            .and_then(|arr| {
                Some(
                    arr.into_iter()
                        .map(|v| v["prioritizationFee"].as_u64().unwrap())
                        .collect::<Vec<u64>>(),
                )
            })
            .and_then(|fees| {
                Some(((fees.iter().sum::<u64>() as f32 / fees.len() as f32).ceil() * 1.2) as u64)
            })
            .ok_or_else(|| format!("Failed to parse priority fee response: {:?}", response)),
        FeeStrategy::Triton =>
            serde_json::from_value::<Vec<RpcPrioritizationFee>>(response["result"].clone())
                .map(|prioritization_fees| {
                    estimate_prioritization_fee_micro_lamports(prioritization_fees)
                })
                .or_else(|error: serde_json::Error| {
                    Err(format!(
                        "Failed to parse priority fee response: {response:?}, error: {error}"
                    ))
                }),
        FeeStrategy::LOCAL => local_dynamic_fee(rpc_client)
            .await
            .or_else(|err| Err(format!("Failed to parse priority fee response: {err}"))),
    };

    // Check if the calculated fee is higher than max
    match calculated_fee {
        Err(err) => Err(err),
        Ok(fee) => {
            if let Some(max_fee) = priority_fee_cap {
                // Ok((fee + 5000).min(max_fee)) // add extra 5000 microlamports as buffer
                Ok((fee).min(max_fee))
            } else {
                // Ok(fee + 5000)
                Ok(fee)
            }
        },
    }
}

pub async fn local_dynamic_fee(rpc_client: &RpcClient) -> Result<u64, Box<dyn std::error::Error>> {
    let client = rpc_client;
    let pubkey = [
        "oreV2ZymfyeXgNgBdqMkumTqqAprVqgBWQfoYkrtKWQ",
        "5HngGmYzvSuh3XyU11brHDpMTHXQQRQQT4udGFtQSjgR",
        "2oLNTQKRb4a2117kFi6BYTUDu3RPrMVAHFhCfPKMosxX",
    ];
    let address_strings = pubkey;

    // Convert strings to Pubkey
    let addresses: Vec<Pubkey> = address_strings
        .into_iter()
        .map(|addr_str| Pubkey::from_str(addr_str).expect("Invalid address"))
        .collect();

    // Get recent prioritization fees
    let recent_prioritization_fees = client.get_recent_prioritization_fees(&addresses).await?;
    if recent_prioritization_fees.is_empty() {
        return Err("No recent prioritization fees".into());
    }
    let mut sorted_fees: Vec<_> = recent_prioritization_fees.into_iter().collect();
    sorted_fees.sort_by(|a, b| b.slot.cmp(&a.slot));
    let chunk_size = 150;
    let chunks: Vec<_> = sorted_fees.chunks(chunk_size).take(3).collect();
    let mut percentiles: HashMap<u8, u64> = HashMap::new();
    for (_, chunk) in chunks.iter().enumerate() {
        let fees: Vec<u64> = chunk.iter().map(|fee| fee.prioritization_fee).collect();
        percentiles = calculate_percentiles(&fees);
    }

    // Default to 75 percentile
    let fee = *percentiles.get(&75).unwrap_or(&0);
    Ok(fee)
}

fn calculate_percentiles(fees: &[u64]) -> HashMap<u8, u64> {
    let mut sorted_fees = fees.to_vec();
    sorted_fees.sort_unstable();
    let len = sorted_fees.len();
    let percentiles = vec![10, 25, 50, 60, 70, 75, 80, 85, 90, 100];
    percentiles
        .into_iter()
        .map(|p| {
            let index = (p as f64 / 100.0 * len as f64).round() as usize;
            (p, sorted_fees[index.saturating_sub(1)])
        })
        .collect()
}

/// Our estimate is the average over the last 20 slots
pub fn estimate_prioritization_fee_micro_lamports(
    prioritization_fees: Vec<RpcPrioritizationFee>,
) -> u64 {
    let prioritization_fees = prioritization_fees
        .into_iter()
        .rev()
        .take(20)
        .map(|RpcPrioritizationFee { prioritization_fee, .. }| prioritization_fee)
        .collect::<Vec<_>>();
    if prioritization_fees.is_empty() {
        panic!("Response does not contain any prioritization fees");
    }

    let prioritization_fee =
        prioritization_fees.iter().sum::<u64>() / prioritization_fees.len() as u64;

    prioritization_fee
}