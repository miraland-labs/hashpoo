pub use ore_utils::AccountDeserialize;
use {
    cached::proc_macro::cached,
    drillx::Solution,
    ore_api::{
        consts::{
            BUS_ADDRESSES, BUS_COUNT, CONFIG_ADDRESS, MINT_ADDRESS, PROOF, TOKEN_DECIMALS,
            TREASURY_ADDRESS,
        },
        state::{Bus, Config, Proof},
    },
    rand::Rng,
    solana_client::nonblocking::rpc_client::RpcClient,
    solana_sdk::{clock::Clock, instruction::Instruction, pubkey::Pubkey, sysvar},
    spl_associated_token_account::get_associated_token_address,
    std::{fs, io::Cursor, time::Duration},
    tracing::{error, info},
};

pub const ORE_TOKEN_DECIMALS: u8 = TOKEN_DECIMALS;

pub fn get_auth_ix(signer: Pubkey) -> Instruction {
    // let proof = proof_pubkey(signer);
    let proof = solo_proof_pubkey(signer);

    ore_api::instruction::auth(proof)
}

// MI: add for pool operation
pub fn _get_pool_auth_ix(signer: Pubkey) -> Instruction {
    let proof = mini_pool_proof_pubkey(signer);

    ore_api::instruction::auth(proof)
}

pub fn get_mine_ix(signer: Pubkey, solution: Solution, bus: usize) -> Instruction {
    ore_api::instruction::mine(signer, signer, BUS_ADDRESSES[bus], solution)
}

pub fn get_register_ix(signer: Pubkey) -> Instruction {
    ore_api::instruction::open(signer, signer, signer)
}

// // MI: add for auto launch if necessary
// pub fn _get_pool_register_ix(signer: Pubkey, pool_url: String) -> Instruction {
//     ore_pool_api::sdk::launch(signer, signer, pool_url).unwrap()
// }

pub fn get_reset_ix(signer: Pubkey) -> Instruction {
    ore_api::instruction::reset(signer)
}

// pub fn _get_pool_claim_ix(signer: Pubkey, beneficiary: Pubkey, claim_amount: u64) -> Instruction {
//     // let (pool_pda, bump) = ore_pool_api::state::pool_pda(operator.keypair.pubkey());
//     let (pool_pda_address, pool_bump) = ore_pool_api::state::pool_pda(signer);
//     ore_pool_api::sdk::claim(signer, beneficiary, pool_pda_address, pool_bump, claim_amount)
// }

pub fn get_claim_ix(signer: Pubkey, beneficiary: Pubkey, claim_amount: u64) -> Instruction {
    ore_api::instruction::claim(signer, beneficiary, claim_amount)
}

pub fn get_ore_mint() -> Pubkey {
    MINT_ADDRESS
}

// MI: used in legacy multiplier calc. which is sunset now
pub async fn _get_config(client: &RpcClient) -> Result<ore_api::state::Config, String> {
    loop {
        let data = client.get_account_data(&CONFIG_ADDRESS).await;
        match data {
            Ok(data) => {
                let config = Config::try_from_bytes(&data);
                if let Ok(config) = config {
                    return Ok(*config);
                } else {
                    return Err("Failed to parse config account data".to_string());
                }
            },
            Err(e) => {
                // return Err("Failed to get config account".to_string());
                error!(target: "server_log", "Failed to get config account: {:?}", e);
                info!(target: "server_log", "Retry to get config account...");
            },
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// MI
pub async fn _get_proof_and_best_bus(
    client: &RpcClient,
    authority: Pubkey,
) -> Result<(Proof, (/* bus index: */ usize, /* bus address: */ Pubkey)), ()> {
    let account_pubkeys = vec![
        solo_proof_pubkey(authority),
        BUS_ADDRESSES[0],
        BUS_ADDRESSES[1],
        BUS_ADDRESSES[2],
        BUS_ADDRESSES[3],
        BUS_ADDRESSES[4],
        BUS_ADDRESSES[5],
        BUS_ADDRESSES[6],
        BUS_ADDRESSES[7],
    ];
    let accounts = client.get_multiple_accounts(&account_pubkeys).await;
    if let Ok(accounts) = accounts {
        let proof = if let Some(account) = &accounts[0] {
            *Proof::try_from_bytes(&account.data).expect("Failed to parse proof account")
        } else {
            return Err(());
        };

        // Fetch the bus with the largest balance
        let mut top_bus_balance: u64 = 0;
        let mut top_bus_id = 0;
        let mut top_bus = BUS_ADDRESSES[0];
        for account in &accounts[1..] {
            if let Some(account) = account {
                if let Ok(bus) = Bus::try_from_bytes(&account.data) {
                    if bus.rewards.gt(&top_bus_balance) {
                        top_bus_balance = bus.rewards;
                        top_bus_id = bus.id as usize;
                        top_bus = BUS_ADDRESSES[top_bus_id];
                    }
                } else {
                    return Err(());
                }
            }
        }

        Ok((proof, (top_bus_id, top_bus)))
    } else {
        Err(())
    }
}

// MI
pub async fn get_config_and_proof(
    client: &RpcClient,
    authority: Pubkey,
) -> Result<(Config, Proof), ()> {
    let account_pubkeys = vec![CONFIG_ADDRESS, solo_proof_pubkey(authority)];
    let accounts = client.get_multiple_accounts(&account_pubkeys).await;
    if let Ok(accounts) = accounts {
        let config = if let Some(account) = &accounts[0] {
            *Config::try_from_bytes(&account.data).expect("Failed to parse config account")
        } else {
            return Err(());
        };

        let proof = if let Some(account) = &accounts[1] {
            *Proof::try_from_bytes(&account.data).expect("Failed to parse proof account")
        } else {
            return Err(());
        };

        Ok((config, proof))
    } else {
        Err(())
    }
}

// MI
async fn _find_bus(rpc_client: &RpcClient) -> Pubkey {
    // Fetch the bus with the largest balance
    if let Ok(accounts) = rpc_client.get_multiple_accounts(&BUS_ADDRESSES).await {
        let mut top_bus_balance: u64 = 0;
        let mut top_bus = BUS_ADDRESSES[0];
        for account in accounts {
            if let Some(account) = account {
                if let Ok(bus) = Bus::try_from_bytes(&account.data) {
                    if bus.rewards.gt(&top_bus_balance) {
                        top_bus_balance = bus.rewards;
                        top_bus = BUS_ADDRESSES[bus.id as usize];
                    }
                }
            }
        }
        return top_bus;
    }

    // Otherwise return a random bus
    let i = rand::thread_rng().gen_range(0..BUS_COUNT);
    BUS_ADDRESSES[i]
}

// Fetch proof account by address
pub async fn get_proof(client: &RpcClient, address: Pubkey) -> Result<Proof, String> {
    let mut retries = 0;
    loop {
        let data = client.get_account_data(&address).await;
        match data {
            Ok(data) => {
                let proof = Proof::try_from_bytes(&data);
                if let Ok(proof) = proof {
                    return Ok(*proof);
                } else {
                    return Err("Failed to parse proof account data".to_string());
                }
            },
            Err(e) => {
                if retries >= 4 {
                    return Err("Failed 5 attempts to get proof account. The proof account has not been created yet.".to_string());
                }
                retries += 1;
                error!(target: "server_log", "Failed to get proof account: {:?}", e);
                info!(target: "server_log", "Retry to get proof account...");
            },
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// Fetch solo proof account for authority/miner in loop
pub async fn get_solo_proof(client: &RpcClient, authority: Pubkey) -> Result<Proof, String> {
    let mut retries = 0;
    loop {
        // let proof_address = proof_pubkey(authority);
        let proof_address = solo_proof_pubkey(authority);
        let data = client.get_account_data(&proof_address).await;
        match data {
            Ok(data) => {
                let proof = Proof::try_from_bytes(&data);
                if let Ok(proof) = proof {
                    return Ok(*proof);
                } else {
                    return Err("Failed to parse solo proof account data".to_string());
                }
            },
            Err(e) => {
                // return Err("Failed to get proof account".to_string()),
                if retries >= 4 {
                    return Err("Failed 5 attempts to get solo proof account. The proof account has not been created yet.".to_string());
                }
                retries += 1;
                error!(target: "server_log", "Failed to get solo proof account: {:?}", e);
                info!(target: "server_log", "Retry to get solo proof account...");
            },
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// Fetch mini pool proof account for pool operator in loop
pub async fn get_mini_pool_proof(client: &RpcClient, operator: Pubkey) -> Result<Proof, String> {
    get_solo_proof(client, operator).await
}

// // Fetch pool proof account for pool operator in loop
// pub async fn get_pool_proof(client: &RpcClient, operator: Pubkey) -> Result<Proof, String> {
//     loop {
//         let proof_address = pool_proof_pubkey(operator);
//         let data = client.get_account_data(&proof_address).await;
//         match data {
//             Ok(data) => {
//                 let proof = Proof::try_from_bytes(&data);
//                 if let Ok(proof) = proof {
//                     return Ok(*proof);
//                 } else {
//                     return Err("Failed to parse pool proof account data".to_string());
//                 }
//             },
//             Err(e) => {
//                 // return Err("Failed to get proof account".to_string()),
//                 error!(target: "server_log", "Failed to get pool proof account: {:?}", e);
//                 info!(target: "server_log", "Retry to get pool proof account...");
//             },
//         }
//         tokio::time::sleep(Duration::from_millis(500)).await;
//     }
// }

pub async fn get_clock(client: &RpcClient) -> Clock {
    let data: Vec<u8>;
    loop {
        match client.get_account_data(&sysvar::clock::ID).await {
            Ok(d) => {
                data = d;
                break;
            },
            Err(e) => {
                error!(target: "server_log", "get clock account error: {:?}", e);
                info!(target: "server_log", "retry to get clock account...");
            },
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    bincode::deserialize::<Clock>(&data).expect("Failed to deserialize clock")
}

// MI: use on-chain clock time without risk time version
pub async fn get_cutoff(rpc_client: &RpcClient, proof: Proof, buffer_time: u64) -> i64 {
    let clock = get_clock(rpc_client).await;
    proof.last_hash_at + 60 as i64 - buffer_time as i64 - clock.unix_timestamp
}

// MI: use on-chain clock time with risk time(default 0) version
pub async fn get_cutoff_with_risk(
    rpc_client: &RpcClient,
    proof: Proof,
    buffer_time: u64,
    risk_time: u64,
) -> i64 {
    let clock = get_clock(rpc_client).await;
    proof.last_hash_at + 60 as i64 + risk_time as i64 - buffer_time as i64 - clock.unix_timestamp
}

// MI
// #[cached]
pub fn play_sound() {
    match rodio::OutputStream::try_default() {
        Ok((_stream, handle)) => {
            let sink = rodio::Sink::try_new(&handle).unwrap();
            let bytes = include_bytes!("../assets/success.mp3");
            let cursor = Cursor::new(bytes);
            sink.append(rodio::Decoder::new(cursor).unwrap());
            sink.sleep_until_end();
        },
        Err(_) => print!("\x07"),
    }
}

pub fn normalized_hashpower(
    unit_hashpower: u64,
    base: u32,
    diff: u32,
    power_cap: Option<u64>,
) -> u64 {
    let mut hashpower = unit_hashpower * 2u64.pow(diff - base);
    if let Some(power_cap) = power_cap {
        if hashpower > power_cap {
            hashpower = power_cap;
        }
    }

    hashpower
}

pub fn _exists_file(file_path: &str) -> bool {
    // This function checks if a file (like db file) exists at the specified file path
    fs::metadata(file_path).is_ok()
}

#[deprecated(since = "0.1.0", note = "Use `solo_proof_pubkey` instead")]
#[cached]
pub fn proof_pubkey(authority: Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[PROOF, authority.as_ref()], &ore_api::ID).0
}

#[cached]
pub fn treasury_tokens_pubkey() -> Pubkey {
    get_associated_token_address(&TREASURY_ADDRESS, &MINT_ADDRESS)
}

#[cached]
pub fn solo_proof_pubkey(authority: Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[PROOF, authority.as_ref()], &ore_api::ID).0
}

#[cached]
pub fn mini_pool_proof_pubkey(authority: Pubkey) -> Pubkey {
    solo_proof_pubkey(authority)
}

#[cached]
pub fn mini_pool_pubkey(authority: Pubkey) -> Pubkey {
    authority
}

// #[cached]
// pub fn pool_proof_pubkey(authority: Pubkey) -> Pubkey {
//     // program_id = ore_pool_api::id()
//     // authority is pool operator
//     let (pool_pda, _pool_bump) = ore_pool_api::state::pool_pda(authority);
//     // program_id = ore_api::id()
//     let (pool_proof_pda, _pool_proof_bump) = ore_pool_api::state::pool_proof_pda(pool_pda);

//     // return the operator's pool proof pda address instead of the operator's own proof pda
// address     pool_proof_pda
// }

// #[cached]
// pub fn pool_pubkey(authority: Pubkey) -> Pubkey {
//     // program_id = ore_pool_api::id()
//     // authority is pool operator
//     let (pool_pda, _pool_bump) = ore_pool_api::state::pool_pda(authority);

//     // return the operator's pool pda account address
//     pool_pda
// }
