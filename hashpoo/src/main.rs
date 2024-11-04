#[cfg(feature = "powered-by-dbms-postgres")]
use processors::claim_processor::claim_processor;
use {
    self::models::*,
    // ::ore_utils::AccountDeserialize,
    axum::{
        debug_handler,
        extract::{
            ws::{Message, WebSocket},
            ConnectInfo, Query, State, WebSocketUpgrade,
        },
        http::{Method, Response, StatusCode},
        response::IntoResponse,
        routing::{get, post},
        Extension, Json, Router,
    },
    axum_extra::{headers::authorization::Basic, TypedHeader},
    base64::{prelude::BASE64_STANDARD, Engine},
    bitflags::bitflags,
    clap::{
        builder::{
            styling::{AnsiColor, Effects},
            Styles,
        },
        command, Parser,
    },
    database::{Database, DatabaseError, PoweredByDbms, PoweredByParams},
    drillx::Solution,
    dynamic_fee as pfee,
    futures::{stream::SplitSink, StreamExt},
    notification::RewardsMessage,
    ore_api::consts::EPOCH_DURATION,
    processors::{
        client_message_processor::client_message_processor,
        messaging_all_clients_processor::messaging_all_clients_processor,
        ping_check_processor::ping_check_processor,
        pong_tracking_processor::pong_tracking_processor,
        pool_mine_success_processor::pool_mine_success_processor,
        pool_submission_processor::pool_submission_processor,
        proof_tracking_processor::proof_tracking_processor,
        ready_clients_processor::ready_clients_processor, reporting_processor::reporting_processor,
    },
    routes::{get_challenges, get_latest_mine_transaction, get_pool_balance},
    rr_database::RrDatabase,
    serde::Deserialize,
    solana_client::nonblocking::rpc_client::RpcClient,
    solana_sdk::{
        commitment_config::{CommitmentConfig, CommitmentLevel},
        native_token::{lamports_to_sol, LAMPORTS_PER_SOL},
        pubkey::Pubkey,
        signature::{read_keypair_file, Keypair, Signature},
        signer::Signer,
        transaction::Transaction,
    },
    spl_associated_token_account::get_associated_token_address,
    std::{
        collections::{HashMap, HashSet},
        net::SocketAddr,
        ops::ControlFlow,
        path::Path,
        str::FromStr,
        sync::{atomic::AtomicBool, Arc, Once, OnceLock},
        time::{SystemTime, UNIX_EPOCH},
    },
    tokio::{
        sync::{mpsc::UnboundedSender, Mutex, RwLock},
        time::Instant,
    },
    tower_http::{
        cors::CorsLayer,
        trace::{DefaultMakeSpan, TraceLayer},
    },
    tracing::{debug, error, info, warn},
    tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer},
    utils::{
        get_mini_pool_proof, get_ore_mint, get_proof, get_register_ix, mini_pool_proof_pubkey,
        ORE_TOKEN_DECIMALS,
    },
};

mod database;
mod dynamic_fee;
mod message;
mod models;
mod notification;
mod processors;
mod routes;
mod rr_database;
mod tpu;
mod utils;

// [diff]   [hashpower]        [noralized]
// 08       256                1
// 09       512                2
// 10       1024               4
// ...      ...                ...
// 17       131,072            512
// ...      ...                ...
// 21       2,097,152	       8,192
// 22       4,194,304	       16,384
// 23       8,388,608	       32,768
// 24       16,777,216	       65,536
// 25       33,554,432	       131,072
// 26       67,108,864	       262,144
// 27       134,217,728	       524,288
// 28       268,435,456	       1,048,576
// 29       536,870,912	       2,097,152
// 30       1,073,741,824      4,194,304
#[allow(dead_code)]
const HASHPOWER_CAP: u64 = 32_768; // map to diff 23
const MIN_DIFF: u32 = 8; // corresponds to current ORE program config
const UNIT_HASHPOWER: u64 = 1;
// MI
// min hash power is matching with ore BASE_REWARD_RATE_MIN_THRESHOLD
// min difficulty, matching with MIN_HASHPOWER.
// const MIN_HASHPOWER: u64 = 5;

// 0.00500000000 ORE
const MIN_CLAIM_AMOUNT_NOT_EXISTS_ATA: u64 = 500_000_000; // grains

// 0.0000500000000 ORE
const MIN_CLAIM_AMOUNT_EXISTS_ATA: u64 = 5_000_000; // grains

// 0.00400000000 ORE
const CREATE_ATA_DEDUCTION: u64 = 400_000_000; // grains
                                               // const CREATE_ATA_DEDUCTION: u64 = 100_000; // for test

// MI: if 0, rpc node will retry the tx until it is finalized or until the blockhash expires
const RPC_RETRIES: usize = 3; // 5

const SUBMIT_LIMIT: u32 = 5;
const CHECK_LIMIT: usize = 30; // 30
const NO_BEST_SOLUTION_INTERVAL: usize = 5;

static POWERED_BY_DBMS: OnceLock<PoweredByDbms> = OnceLock::new();
static WALLET_PUBKEY: OnceLock<Pubkey> = OnceLock::new();
static PAUSED: AtomicBool = AtomicBool::new(false);

static mut MESSAGING_FLAGS: MessagingFlags = MessagingFlags::empty();
static INIT_MESSAGING_FLAGS: Once = Once::new();

static mut SLACK_WEBHOOK: String = String::new();
static mut DISCORD_WEBHOOK: String = String::new();

fn get_messaging_flags() -> MessagingFlags {
    unsafe {
        INIT_MESSAGING_FLAGS.call_once(|| {
            let mut exists_slack_webhook = false;

            let key = "SLACK_WEBHOOK";
            match std::env::var(key) {
                Ok(val) => {
                    exists_slack_webhook = true;
                    SLACK_WEBHOOK = val;
                },
                Err(e) => {
                    warn!(target: "server_log", "couldn't interpret {key}: {e}. slack messaging service unvailable.")
                },
            }

            let mut exists_discord_webhook = false;
            let key = "DISCORD_WEBHOOK";
            match std::env::var(key) {
                Ok(val) => {
                    exists_discord_webhook = true;
                    DISCORD_WEBHOOK = val;
                },
                Err(e) => {
                    warn!(target: "server_log", "couldn't interpret {key}: {e}. discord messaging service unvailable.")
                },
            }

            let mut messaging_flags = MessagingFlags::empty();
            if exists_slack_webhook {
                messaging_flags |= MessagingFlags::SLACK;
            }
            if exists_discord_webhook {
                messaging_flags |= MessagingFlags::DISCORD;
            }

            MESSAGING_FLAGS = messaging_flags;
        });
        MESSAGING_FLAGS
    }
}

#[derive(Clone)]
enum ClientVersion {
    #[allow(dead_code)]
    V0,
    V1,
}

#[derive(Clone)]
struct ClientConnection {
    pubkey: Pubkey,
    miner_id: i64,
    client_version: ClientVersion,
    socket: Arc<Mutex<SplitSink<WebSocket, Message>>>,
}

struct WalletExtension {
    miner_wallet: Arc<Keypair>,
    #[allow(dead_code)]
    fee_wallet: Arc<Keypair>,
}

struct AppState {
    sockets: HashMap<SocketAddr, ClientConnection>,
}

#[derive(Clone, Copy)]
struct ClaimsQueueItem {
    receiver_pubkey: Pubkey,
    amount: u64,
}

struct ClaimsQueue {
    queue: RwLock<HashMap<Pubkey, ClaimsQueueItem>>,
}

pub struct MessageInternalAllClients {
    text: String,
}

#[derive(Debug, Clone, Copy)]
pub struct InternalMessageContribution {
    miner_id: i64,
    supplied_diff: u32,
    supplied_digest: [u8; 16], // MI added
    supplied_nonce: u64,
    hashpower: u64,
}

pub struct MessageInternalMineSuccess {
    difficulty: u32,
    total_balance: f64,
    rewards: i64,
    commissions: i64,
    challenge_id: i64,
    challenge: [u8; 32],
    best_nonce: u64,
    total_hashpower: u64,
    ore_config: Option<ore_api::state::Config>,
    multiplier: f64,
    contributions: HashMap<Pubkey, InternalMessageContribution>,
}

pub struct LastPong {
    pongs: HashMap<SocketAddr, Instant>,
}

#[derive(Debug)]
pub enum ClientMessage {
    Ready(SocketAddr),
    Mining(SocketAddr),
    Pong(SocketAddr),
    BestSolution(SocketAddr, Solution, Pubkey),
}

pub struct EpochHashes {
    challenge: [u8; 32],
    best_hash: BestHash,
    contributions: HashMap<Pubkey, InternalMessageContribution>,
}

pub struct BestHash {
    solution: Option<Solution>,
    difficulty: u32,
}

pub struct MineConfig {
    // mining pool db table rowid/identity if powered by dbms
    pool_id: i32,
    stats_enabled: bool,
    #[allow(dead_code)]
    commissions_pubkey: String,
    commissions_miner_id: i64,
}

bitflags! {
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct MessagingFlags: u8 {
        const SLACK   = 1 << 0;
        const DISCORD = 1 << 1;
        // const EMAIL   = 1 << 2;
    }
}

pub struct DifficultyPayload {
    pub solution_difficulty: u32,
    pub expected_min_difficulty: u32,
    pub extra_fee_difficulty: u32,
    pub extra_fee_percent: u64,
}

#[derive(Parser, Debug)]
#[command(version, author, about, long_about = None, styles = styles())]
struct Args {
    #[arg(
        long,
        short,
        value_name = "BUFFER_SECONDS",
        help = "The number seconds before the deadline to stop mining and start submitting.",
        default_value = "5"
    )]
    pub buffer_time: u64,

    #[arg(
        long,
        short,
        value_name = "RISK_SECONDS",
        help = "Set extra hash time in seconds for miners to stop mining and start submitting, risking a penalty.",
        default_value = "0"
    )]
    pub risk_time: u64,

    #[arg(
        long,
        value_name = "FEE_MICROLAMPORTS",
        help = "Price to pay for compute units when dynamic fee flag is off, or dynamic fee is unavailable.",
        default_value = "100",
        global = true
    )]
    priority_fee: Option<u64>,

    #[arg(
        long,
        value_name = "FEE_CAP_MICROLAMPORTS",
        help = "Max price to pay for compute units when dynamic fees are enabled.",
        default_value = "100000",
        global = true
    )]
    priority_fee_cap: Option<u64>,

    #[arg(long, help = "Enable dynamic priority fees", global = true)]
    dynamic_fee: bool,

    #[arg(long, short, action, help = "Enable stats endpoints")]
    stats: bool,

    #[arg(
        long,
        value_name = "DYNAMIC_FEE_URL",
        help = "RPC URL to use for dynamic fee estimation.",
        global = true
    )]
    dynamic_fee_url: Option<String>,

    #[arg(
        long,
        short,
        value_name = "EXPECTED_MIN_DIFFICULTY",
        help = "The expected min difficulty to submit from pool client. Reserved for potential qualification process unimplemented yet.",
        default_value = "8"
    )]
    pub expected_min_difficulty: u32,

    #[arg(
        long,
        short,
        value_name = "EXTRA_FEE_DIFFICULTY",
        help = "The min difficulty that the pool server miner thinks deserves to pay more priority fee to land tx quickly.",
        default_value = "29"
    )]
    pub extra_fee_difficulty: u32,

    #[arg(
        long,
        short,
        value_name = "EXTRA_FEE_PERCENT",
        help = "The extra percentage that the pool server miner feels deserves to pay more of the priority fee. As a percentage, a multiple of 50 is recommended(example: 50, means pay extra 50% of the specified priority fee), and the final priority fee cannot exceed the priority fee cap.",
        default_value = "0"
    )]
    pub extra_fee_percent: u64,

    #[arg(
        long,
        short,
        value_name = "SLACK_DIFFICULTY",
        help = "The min difficulty that will notify slack channel(if configured) upon transaction success. It's deprecated in favor of messaging_diff",
        default_value = "25"
    )]
    pub slack_difficulty: u32,

    #[arg(
        long,
        short,
        value_name = "MESSAGING_DIFF",
        help = "The min difficulty that will notify messaging channels(if configured) upon transaction success.",
        default_value = "25"
    )]
    pub messaging_diff: u32,

    #[arg(long, help = "Send and confirm transactions using tpu client.", global = true)]
    send_tpu_mine_tx: bool,

    /// Mine with sound notification on/off
    #[arg(
        long,
        value_name = "NO_SOUND_NOTIFICATION",
        help = "Sound notification on by default",
        default_value = "false",
        global = true
    )]
    pub no_sound_notification: bool,
}

// #[tokio::main(flavor = "multi_thread", worker_threads = 12)]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    color_eyre::install().unwrap();
    dotenvy::dotenv().ok();
    let args = Args::parse();

    let log_file_dir: String;
    let key = "LOG_FILE_DIR";
    match std::env::var(key) {
        Ok(val) => {
            log_file_dir = val;
        },
        Err(_) => log_file_dir = String::from("./logs"),
    }

    // MI: pure env filter
    // let env_filter_layer = tracing_subscriber::EnvFilter::try_from_default_env()
    //     .or_else(|_| tracing_subscriber::EnvFilter::try_new("info"))
    //     .unwrap();
    let env_filter_stdout = tracing_subscriber::EnvFilter::try_from_default_env()
        .or_else(|_| tracing_subscriber::EnvFilter::try_new("info"))
        .unwrap();

    // MI: complete layer definition
    let stdout_log_layer = tracing_subscriber::fmt::layer()
        .compact()
        // .pretty()
        .with_filter(env_filter_stdout)
        .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
            metadata.target() == "server_log"
        }));

    let env_filter_srvlog = tracing_subscriber::EnvFilter::try_from_default_env()
        .or_else(|_| tracing_subscriber::EnvFilter::try_new("trace"))
        .unwrap();

    // MI: complete layer definition
    // let server_logs = tracing_appender::rolling::daily("./logs", "hashpoo.log");
    let server_logs = tracing_appender::rolling::daily(&log_file_dir, "hashpoo.log");
    let (server_logs, _guard) = tracing_appender::non_blocking(server_logs);
    let server_log_layer = tracing_subscriber::fmt::layer()
        .with_writer(server_logs)
        .with_filter(env_filter_srvlog)
        .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
            metadata.target() == "server_log"
        }));

    let env_filter_conlog = tracing_subscriber::EnvFilter::try_from_default_env()
        .or_else(|_| tracing_subscriber::EnvFilter::try_new("trace"))
        .unwrap();
    // MI: complete layer definition
    // let contribution_logs = tracing_appender::rolling::daily("./logs", "contributions.log");
    let contribution_logs = tracing_appender::rolling::daily(&log_file_dir, "contributions.log");
    let (contribution_logs, _guard) = tracing_appender::non_blocking(contribution_logs);
    let contribution_log_layer = tracing_subscriber::fmt::layer()
        .with_writer(contribution_logs)
        .with_filter(env_filter_conlog)
        .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
            metadata.target() == "contribution_log"
        }));

    tracing_subscriber::registry()
        // .with(fmt::layer()) // MI: default layer() which is only required when followed by pure
        // EnvFilter w/o layer definition
        .with(stdout_log_layer)
        .with(server_log_layer)
        .with(contribution_log_layer)
        .init();

    // let file_appender = tracing_appender::rolling::daily("./logs", "ore-ppl-srv.log");
    // let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    // tracing_subscriber::fmt().with_env_filter(filter_layer).with_writer(non_blocking).init();

    // load envs
    let wallet_path_str = std::env::var("WALLET_PATH").expect("WALLET_PATH must be set.");
    let key = "FEE_WALLET_PATH";
    let fee_wallet_path_str = match std::env::var(key) {
        Ok(val) => val,
        Err(_) => {
            info!(target: "server_log", "FEE_WALLET_PATH not set, using WALLET_PATH instead.");
            wallet_path_str.clone()
        },
    };
    let rpc_url = std::env::var("RPC_URL").expect("RPC_URL must be set.");
    let rpc_ws_url = std::env::var("RPC_WS_URL").expect("RPC_WS_URL must be set.");

    let powered_by_dbms = POWERED_BY_DBMS.get_or_init(|| {
        let key = "POWERED_BY_DBMS";
        match std::env::var(key) {
            Ok(val) => {
                PoweredByDbms::from_str(&val).expect("POWERED_BY_DBMS must be set correctly.")
            },
            Err(_) => PoweredByDbms::Unavailable,
        }
    });

    let database_uri: String;
    let key = "DATABASE_URL";
    match std::env::var(key) {
        Ok(val) => {
            database_uri = val;
        },
        Err(_) => database_uri = String::from("ore_priv_pool.db.sqlite3"),
    }

    let database_rr_uri = std::env::var("DATABASE_RR_URL").expect("DATABASE_RR_URL must be set.");
    let commission_env =
        std::env::var("COMMISSION_PUBKEY").expect("COMMISSION_PUBKEY must be set.");
    let commission_pubkey = match Pubkey::from_str(&commission_env) {
        Ok(pk) => pk,
        Err(_) => {
            println!("Invalid COMMISSION_PUBKEY");
            return Ok(());
        },
    };

    let reports_interval_in_hrs: u64 = match std::env::var("REPORTS_INTERVAL_IN_HOURS") {
        Ok(val) => val.parse().expect("REPORTS_INTERVAL_IN_HOURS must be a positive number"),
        Err(_) => 6,
    };

    let mut dbms_settings = PoweredByParams {
        // default to "./ore_priv_pool.db.sqlite3" for sqlite
        database_uri: &database_uri,
        database_rr_uri: &database_rr_uri,
        initialized: false,
        corrupted: false,
    };

    if powered_by_dbms == &PoweredByDbms::Postgres || powered_by_dbms == &PoweredByDbms::Sqlite {
        info!(target: "server_log", "Powered by {} detected.", powered_by_dbms);
    }

    let database = Database::new(dbms_settings.database_uri.into());

    if (powered_by_dbms == &PoweredByDbms::Postgres || powered_by_dbms == &PoweredByDbms::Sqlite)
        && !database::validate_dbms(&mut dbms_settings, database.clone()).await
    {
        return Err("ORE mining pool db failed validation.".into());
    }

    let rr_database = Arc::new(RrDatabase::new(dbms_settings.database_rr_uri.into()));

    let database = Arc::new(database);

    let messaging_flags = get_messaging_flags();

    let priority_fee = Arc::new(args.priority_fee);
    let priority_fee_cap = Arc::new(args.priority_fee_cap);

    let buffer_time = Arc::new(args.buffer_time);
    let risk_time = Arc::new(args.risk_time);

    let min_difficulty = Arc::new(args.expected_min_difficulty);
    let extra_fee_difficulty = Arc::new(args.extra_fee_difficulty);
    let extra_fee_percent = Arc::new(args.extra_fee_percent);

    let dynamic_fee = Arc::new(args.dynamic_fee);
    let dynamic_fee_url = Arc::new(args.dynamic_fee_url);

    let slack_difficulty = Arc::new(args.slack_difficulty);
    let messaging_diff = Arc::new(args.messaging_diff);

    let send_tpu_mine_tx = Arc::new(args.send_tpu_mine_tx);

    let no_sound_notification = Arc::new(args.no_sound_notification);

    // load wallet
    let wallet_path = Path::new(&wallet_path_str);

    if !wallet_path.exists() {
        tracing::error!(target: "server_log", "❌ Failed to load wallet at: {}", wallet_path_str);
        return Err("Failed to find wallet path.".into());
    }

    let wallet = read_keypair_file(wallet_path)
        .expect("Failed to load keypair from file: {wallet_path_str}");
    let wallet_pubkey = wallet.pubkey();
    info!(target: "server_log", "loaded wallet {}", wallet_pubkey.to_string());

    // load fee wallet
    let wallet_path = Path::new(&fee_wallet_path_str);

    if !wallet_path.exists() {
        tracing::error!(target: "server_log", "❌ Failed to load fee wallet at: {}", fee_wallet_path_str);
        return Err("Failed to find fee wallet path.".into());
    }

    let fee_wallet = read_keypair_file(wallet_path)
        .expect("Failed to load keypair from file: {wallet_path_str}");
    info!(target: "server_log", "loaded fee wallet {}", wallet.pubkey().to_string());

    WALLET_PUBKEY.get_or_init(|| wallet_pubkey);

    info!(target: "server_log", "establishing rpc connection...");
    let rpc_client = RpcClient::new_with_commitment(rpc_url, CommitmentConfig::confirmed());

    info!(target: "server_log", "loading sol balance...");
    let balance = if let Ok(balance) = rpc_client.get_balance(&wallet_pubkey).await {
        balance
    } else {
        return Err("Failed to load balance".into());
    };

    info!(target: "server_log", "Balance: {:.9}", balance as f64 / LAMPORTS_PER_SOL as f64);

    if balance < 1_000_000 {
        return Err("Sol balance is too low!".into());
    }

    let proof_pubkey = mini_pool_proof_pubkey(wallet_pubkey);
    debug!(target: "server_log", "MINI POOL PROOF ADDRESS: {:?}", proof_pubkey);
    let proof = if let Ok(loaded_proof) = get_proof(&rpc_client, proof_pubkey).await {
        debug!(target: "server_log", "LOADED MINI POOL PROOF: \n{:?}", loaded_proof);
        loaded_proof
    } else {
        error!(target: "server_log", "Failed to load mini pool proof.");
        info!(target: "server_log", "Creating mini pool proof account...");

        let ix = get_register_ix(wallet_pubkey);

        if let Ok((hash, _slot)) =
            rpc_client.get_latest_blockhash_with_commitment(rpc_client.commitment()).await
        {
            let mut tx = Transaction::new_with_payer(&[ix], Some(&wallet_pubkey));

            tx.sign(&[&wallet], hash);

            let result = rpc_client
                .send_and_confirm_transaction_with_spinner_and_commitment(
                    &tx,
                    rpc_client.commitment(),
                )
                .await;

            if let Ok(sig) = result {
                info!(target: "server_log", "Sig: {}", sig.to_string());
            } else {
                return Err("Failed to create mini pool proof account".into());
            }
        }
        let proof = if let Ok(loaded_proof) = get_mini_pool_proof(&rpc_client, wallet_pubkey).await
        {
            loaded_proof
        } else {
            return Err("Failed to get newly created mini pool proof".into());
        };
        proof
    };

    let mine_config: Arc<MineConfig>;
    if powered_by_dbms == &PoweredByDbms::Postgres || powered_by_dbms == &PoweredByDbms::Sqlite {
        info!(target: "server_log", "Check if the mining pool record exists in the database");
        let mining_pool = database.get_pool_by_authority_pubkey(wallet_pubkey.to_string()).await;

        match mining_pool {
            Ok(_) => {},
            Err(DatabaseError::FailedToGetConnectionFromPool) => {
                panic!("Failed to get a connection from database pool");
            },
            Err(_) => {
                info!(target: "server_log", "Mining pool record missing from database. Inserting...");
                let proof_pubkey = utils::mini_pool_proof_pubkey(wallet_pubkey);
                let pool_pubkey = utils::mini_pool_pubkey(wallet_pubkey);
                let result = database
                    .add_new_pool(
                        wallet_pubkey.to_string(),
                        proof_pubkey.to_string(),
                        pool_pubkey.to_string(),
                    )
                    .await;

                if result.is_err() {
                    panic!("Failed to add mining pool record in database");
                } else {
                    info!(target: "server_log", "Mining pool record added to database");
                }
            },
        }

        info!(target: "server_log", "Check if the commissions receiver record exists in the database");
        let commission_miner_id;
        match database.get_miner_by_pubkey_str(commission_pubkey.to_string()).await {
            Ok(miner) => {
                info!(target: "server_log", "Found commissions receiver in db.");
                commission_miner_id = miner.id;
            },
            Err(_) => {
                info!(target: "server_log", "Failed to get commissions receiver account from database.");
                info!(target: "server_log", "Inserting Commissions receiver account...");

                match database
                    .signup_enrollment(commission_pubkey.to_string(), wallet_pubkey.to_string())
                    .await
                {
                    Ok(_) => {
                        info!(target: "server_log", "Successfully inserted Commissions receiver account.");
                        if let Ok(m) =
                            database.get_miner_by_pubkey_str(commission_pubkey.to_string()).await
                        {
                            commission_miner_id = m.id;
                        } else {
                            panic!("Failed to get commission receiver account id")
                        }
                    },
                    Err(_) => {
                        panic!("Failed to insert comissions receiver account")
                    },
                }
            },
        }

        let mining_pool =
            database.get_pool_by_authority_pubkey(wallet_pubkey.to_string()).await.unwrap();

        mine_config = Arc::new(MineConfig {
            pool_id: mining_pool.id,
            stats_enabled: args.stats,
            commissions_pubkey: commission_pubkey.to_string(),
            commissions_miner_id: commission_miner_id,
        });

        info!(target: "server_log", "Check if current challenge for pool exists in the database");
        let challenge = database.get_challenge_by_challenge(proof.challenge.to_vec()).await;

        match challenge {
            Ok(_) => {},
            Err(DatabaseError::FailedToGetConnectionFromPool) => {
                panic!("Failed to get a connection from database pool");
            },
            Err(_) => {
                info!(target: "server_log", "Challenge record missing from database. Inserting...");
                let new_challenge = models::InsertChallenge {
                    pool_id: mining_pool.id,
                    challenge: proof.challenge.to_vec(),
                    rewards_earned: None,
                };
                let result = database.add_new_challenge(new_challenge).await;

                if result.is_err() {
                    panic!("Failed to add challenge record in database");
                } else {
                    info!(target: "server_log", "Challenge record added to database");
                }
            },
        }
    } else {
        // NOT POWERED BY DBMS
        mine_config = Arc::new(MineConfig {
            pool_id: i32::MAX,
            stats_enabled: args.stats,
            commissions_pubkey: commission_pubkey.to_string(),
            commissions_miner_id: i64::MAX,
        });
    }

    let epoch_hashes = Arc::new(RwLock::new(EpochHashes {
        challenge: proof.challenge,
        best_hash: BestHash { solution: None, difficulty: 0 },
        contributions: HashMap::new(),
    }));

    let wallet_extension = Arc::new(WalletExtension {
        miner_wallet: Arc::new(wallet),
        fee_wallet: Arc::new(fee_wallet),
    });
    let proof_ext = Arc::new(Mutex::new(proof));
    let nonce_ext = Arc::new(Mutex::new(0u64));

    let client_nonce_ranges = Arc::new(RwLock::new(HashMap::new()));

    let shared_state = Arc::new(RwLock::new(AppState { sockets: HashMap::new() }));
    let ready_clients = Arc::new(Mutex::new(HashSet::new()));

    let pongs = Arc::new(RwLock::new(LastPong { pongs: HashMap::new() }));

    let claims_queue = Arc::new(ClaimsQueue { queue: RwLock::new(HashMap::new()) });

    let rpc_client = Arc::new(rpc_client);

    let last_challenge = Arc::new(Mutex::new([0u8; 32]));

    #[cfg(feature = "powered-by-dbms-postgres")]
    tokio::spawn({
        let rpc_client = rpc_client.clone();
        let wallet = wallet_extension.clone();
        let claims_queue = claims_queue.clone();
        let database = database.clone();
        async move {
            claim_processor(claims_queue, rpc_client, wallet.miner_wallet.clone(), database).await;
        }
    });

    // Track client pong timings
    let app_pongs = pongs.clone();
    let app_state = shared_state.clone();
    tokio::spawn(async move {
        pong_tracking_processor(app_pongs, app_state).await;
    });

    // Establish webocket connection for tracking pool proof changes.
    tokio::spawn({
        let wallet = wallet_extension.clone();
        let proof = proof_ext.clone();
        let last_challenge = last_challenge.clone();
        async move {
            proof_tracking_processor(
                rpc_ws_url,
                wallet.miner_wallet.clone(),
                proof,
                last_challenge,
            )
            .await;
        }
    });

    let (client_message_sender, client_message_receiver) =
        tokio::sync::mpsc::unbounded_channel::<ClientMessage>();

    // Handle client messages
    let app_ready_clients = ready_clients.clone();
    let app_proof = proof_ext.clone();
    let app_epoch_hashes = epoch_hashes.clone();
    let app_client_nonce_ranges = client_nonce_ranges.clone();
    let app_state = shared_state.clone();
    let app_pongs = pongs.clone();
    let app_min_difficulty = min_difficulty.clone();
    tokio::spawn(async move {
        client_message_processor(
            app_state,
            client_message_receiver,
            app_epoch_hashes,
            app_ready_clients,
            app_proof,
            app_client_nonce_ranges,
            app_pongs,
            *app_min_difficulty,
        )
        .await;
    });

    // Handle ready clients
    // rpc has been Arc-ed above
    // let rpc_client = Arc::new(rpc_client);
    let app_rpc_client = rpc_client.clone();
    let app_shared_state = shared_state.clone();
    let app_proof = proof_ext.clone();
    let app_epoch_hashes = epoch_hashes.clone();
    let app_ready_clients = ready_clients.clone();
    let app_nonce = nonce_ext.clone();
    let app_client_nonce_ranges = client_nonce_ranges.clone();
    let app_buffer_time = buffer_time.clone();
    let app_risk_time = risk_time.clone();
    tokio::spawn(async move {
        ready_clients_processor(
            app_rpc_client,
            app_shared_state,
            app_proof,
            app_epoch_hashes,
            app_ready_clients,
            app_nonce,
            app_client_nonce_ranges,
            app_buffer_time,
            app_risk_time,
        )
        .await;
    });

    let (slack_message_sender, slack_message_receiver) =
        tokio::sync::mpsc::unbounded_channel::<RewardsMessage>();

    // if exists slack webhook, handle slack messages to send
    if messaging_flags.contains(MessagingFlags::SLACK) {
        tokio::spawn(async move {
            unsafe {
                notification::slack_messaging_processor(
                    SLACK_WEBHOOK.clone(),
                    slack_message_receiver,
                )
                .await;
            }
        });
    }

    let (discord_message_sender, discord_message_receiver) =
        tokio::sync::mpsc::unbounded_channel::<RewardsMessage>();

    // if exists discord webhook, handle discord messages to send
    if messaging_flags.contains(MessagingFlags::DISCORD) {
        tokio::spawn(async move {
            unsafe {
                notification::discord_messaging_processor(
                    DISCORD_WEBHOOK.clone(),
                    discord_message_receiver,
                )
                .await;
            }
        });
    }

    // Start report routine
    let app_mine_config = mine_config.clone();
    let app_database = database.clone();
    tokio::spawn(async move {
        reporting_processor(reports_interval_in_hrs, app_mine_config, app_database).await;
    });

    let (mine_success_sender, mine_success_receiver) =
        tokio::sync::mpsc::unbounded_channel::<MessageInternalMineSuccess>();

    let (all_clients_sender, all_clients_receiver) =
        tokio::sync::mpsc::unbounded_channel::<MessageInternalAllClients>();

    let app_rpc_client = rpc_client.clone();
    let app_mine_config = mine_config.clone();
    let app_shared_state = shared_state.clone();
    let app_proof = proof_ext.clone();
    let app_epoch_hashes = epoch_hashes.clone();
    let app_wallet = wallet_extension.clone();
    let app_nonce = nonce_ext.clone();
    let app_dynamic_fee = dynamic_fee.clone();
    let app_dynamic_fee_url = dynamic_fee_url.clone();
    let app_priority_fee = priority_fee.clone();
    let app_priority_fee_cap = priority_fee_cap.clone();
    let app_extra_fee_difficulty = extra_fee_difficulty.clone();
    let app_extra_fee_percent = extra_fee_percent.clone();
    let app_send_tpu_mine_tx = send_tpu_mine_tx.clone();
    let app_no_sound_notification = no_sound_notification.clone();
    let app_database = database.clone();
    let app_all_clients_sender = all_clients_sender.clone();
    let app_slack_message_sender = slack_message_sender.clone();
    let app_discord_message_sender = discord_message_sender.clone();
    let app_slack_difficulty = slack_difficulty.clone();
    let app_messaging_diff = messaging_diff.clone();
    let app_buffer_time = buffer_time.clone();
    let app_risk_time = risk_time.clone();
    tokio::spawn(async move {
        pool_submission_processor(
            app_rpc_client,
            app_mine_config,
            app_shared_state,
            app_proof,
            app_epoch_hashes,
            app_wallet,
            app_nonce,
            app_dynamic_fee,
            app_dynamic_fee_url,
            app_priority_fee,
            app_priority_fee_cap,
            app_extra_fee_difficulty,
            app_extra_fee_percent,
            app_send_tpu_mine_tx,
            app_no_sound_notification,
            app_database,
            app_all_clients_sender,
            mine_success_sender,
            app_slack_message_sender,
            app_discord_message_sender,
            app_slack_difficulty,
            app_messaging_diff,
            app_buffer_time,
            app_risk_time,
        )
        .await;
    });

    let app_rpc_client = rpc_client.clone();
    let app_mine_config = mine_config.clone();
    let app_shared_state = shared_state.clone();
    let app_database = database.clone();
    let app_wallet = wallet_extension.clone();
    tokio::spawn(async move {
        pool_mine_success_processor(
            app_rpc_client,
            app_mine_config,
            app_shared_state,
            app_database,
            app_wallet,
            mine_success_receiver,
        )
        .await;
    });

    let app_shared_state = shared_state.clone();
    tokio::spawn(async move {
        messaging_all_clients_processor(app_shared_state, all_clients_receiver).await;
    });

    let cors = CorsLayer::new().allow_methods([Method::GET]).allow_origin(tower_http::cors::Any);

    let client_channel = client_message_sender.clone();
    let app_shared_state = shared_state.clone();
    let app = Router::new()
        .route("/v1/ws", get(ws_handler))
        .route("/v1/latest-blockhash", get(get_latest_blockhash))
        .route("/v1/pool/authority/pubkey", get(get_pool_authority_pubkey))
        .route("/v1/pool/fee_payer/pubkey", get(get_pool_fee_payer_pubkey))
        .route("/v1/signup", post(post_signup))
        .route("/v1/sol-balance", get(get_sol_balance))
        .route("/v1/claim", post(post_claim))
        .route("/v1/active-miners", get(get_connected_miners))
        .route("/timestamp", get(get_timestamp))
        .route("/v1/miner/balance", get(get_miner_balance))
        .route("/v1/stake-multiplier", get(get_stake_multiplier))
        // App RR Database routes
        .route("/v1/last-challenge-contributions", get(get_last_challenge_contributions))
        .route("/v1/miner/rewards", get(get_miner_rewards))
        .route("/v1/miner/contributions", get(get_miner_contributions))
        .route("/v1/miner/last-claim", get(get_miner_last_claim))
        .route("/v1/challenges", get(get_challenges))
        .route("/v1/pool", get(routes::get_pool))
        .route("/v1/pool/staked", get(routes::get_pool_staked))
        .route("/v1/pool/balance", get(get_pool_balance))
        .route("/v1/txns/latest-mine", get(get_latest_mine_transaction))
        .with_state(app_shared_state)
        .layer(Extension(database))
        .layer(Extension(rr_database))
        .layer(Extension(wallet_extension))
        .layer(Extension(client_channel))
        .layer(Extension(rpc_client))
        .layer(Extension(client_nonce_ranges))
        .layer(Extension(claims_queue))
        // Logging
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::default().include_headers(true)),
        )
        .layer(cors);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    tracing::info!(target: "server_log", "listening on {}", listener.local_addr().unwrap());

    let app_shared_state = shared_state.clone();
    tokio::spawn(async move {
        ping_check_processor(&app_shared_state).await;
    });

    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();

    Ok(())
}

async fn get_pool_authority_pubkey(
    Extension(wallet): Extension<Arc<WalletExtension>>,
) -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/text")
        .body(wallet.miner_wallet.pubkey().to_string())
        .unwrap()
}

async fn get_pool_fee_payer_pubkey(
    Extension(wallet): Extension<Arc<WalletExtension>>,
) -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/text")
        .body(wallet.fee_wallet.pubkey().to_string())
        .unwrap()
}

async fn get_latest_blockhash(
    Extension(rpc_client): Extension<Arc<RpcClient>>,
) -> impl IntoResponse {
    let latest_blockhash = rpc_client
        .get_latest_blockhash_with_commitment(CommitmentConfig {
            commitment: CommitmentLevel::Finalized,
        })
        .await
        .unwrap();

    let serialized_blockhash = bincode::serialize(&latest_blockhash).unwrap();

    let encoded_blockhash = BASE64_STANDARD.encode(serialized_blockhash);
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/text")
        .body(encoded_blockhash)
        .unwrap()
}

#[derive(Deserialize)]
struct SignupParams {
    miner: String,
}

async fn post_signup(
    query_params: Query<SignupParams>,
    Extension(database): Extension<Arc<Database>>,
    Extension(wallet): Extension<Arc<WalletExtension>>,
    _body: String,
) -> impl IntoResponse {
    if let Ok(miner_pubkey) = Pubkey::from_str(&query_params.miner) {
        let db_miner = database.get_miner_by_pubkey_str(miner_pubkey.to_string()).await;

        match db_miner {
            Ok(miner) => {
                if miner.enabled {
                    info!(target: "server_log", "Miner account already enabled!");
                    return Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "text/text")
                        .body("EXISTS".to_string())
                        .unwrap();
                }
            },
            Err(DatabaseError::FailedToGetConnectionFromPool) => {
                error!(target: "server_log", "Failed to get database pool connection");
                return Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body("Failed to get db pool connection".to_string())
                    .unwrap();
            },
            Err(_) => {
                info!(target: "server_log", "No miner account exists. Signing up new user.");
            },
        }

        let res = database
            .signup_enrollment(miner_pubkey.to_string(), wallet.miner_wallet.pubkey().to_string())
            .await;

        match res {
            Ok(_) => {
                return Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/text")
                    .body("SUCCESS".to_string())
                    .unwrap();
            },
            Err(_) => {
                error!(target: "server_log", "Failed to add miner to database");
                return Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body("Failed to add user to database".to_string())
                    .unwrap();
            },
        }
    } else {
        error!(target: "server_log", "Signup with invalid miner_pubkey");
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body("Invalid miner pubkey".to_string())
            .unwrap();
    }
}

async fn get_sol_balance(
    Extension(rpc_client): Extension<Arc<RpcClient>>,
    query_params: Query<PubkeyParam>,
) -> impl IntoResponse {
    if let Ok(user_pubkey) = Pubkey::from_str(&query_params.pubkey) {
        let res = rpc_client.get_balance(&user_pubkey).await;

        match res {
            Ok(balance) => {
                let response = format!("{}", lamports_to_sol(balance));
                return Response::builder().status(StatusCode::OK).body(response).unwrap();
            },
            Err(_) => {
                error!(target: "server_log", "get_sol_balance: failed to get sol balance for {}", user_pubkey.to_string());
                return Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body("Failed to get sol balance".to_string())
                    .unwrap();
            },
        }
    } else {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body("Invalid public key".to_string())
            .unwrap();
    }
}

#[derive(Deserialize)]
struct PubkeyParam {
    pubkey: String,
}

async fn get_miner_rewards(
    query_params: Query<PubkeyParam>,
    Extension(rr_database): Extension<Arc<RrDatabase>>,
) -> impl IntoResponse {
    if let Ok(user_pubkey) = Pubkey::from_str(&query_params.pubkey) {
        let res = rr_database.get_miner_rewards(user_pubkey.to_string()).await;

        match res {
            Ok(rewards) => {
                let decimal_bal =
                    rewards.balance as f64 / 10f64.powf(ore_api::consts::TOKEN_DECIMALS as f64);
                let response = format!("{}", decimal_bal);
                return Response::builder().status(StatusCode::OK).body(response).unwrap();
            },
            Err(_) => {
                error!(target: "server_log", "get_miner_rewards: failed to get rewards balance from db for {}", user_pubkey.to_string());
                return Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body("Failed to get balance".to_string())
                    .unwrap();
            },
        }
    } else {
        return Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body("Invalid public key".to_string())
            .unwrap();
    }
}

async fn get_last_challenge_contributions(
    Extension(rr_database): Extension<Arc<RrDatabase>>,
    Extension(mine_config): Extension<Arc<MineConfig>>,
) -> Result<Json<Vec<ContributionWithPubkey>>, String> {
    if mine_config.stats_enabled {
        let res = rr_database.get_last_challenge_contributions().await;

        match res {
            Ok(contributions) => Ok(Json(contributions)),
            Err(_) => Err("Failed to get contributions for miner".to_string()),
        }
    } else {
        return Err("Stats not enabled for this server.".to_string());
    }
}

#[derive(Deserialize)]
struct GetContributionsParams {
    pubkey: String,
}

async fn get_miner_contributions(
    query_params: Query<GetContributionsParams>,
    Extension(rr_database): Extension<Arc<RrDatabase>>,
    Extension(mine_config): Extension<Arc<MineConfig>>,
) -> Result<Json<Vec<Contribution>>, String> {
    if mine_config.stats_enabled {
        if let Ok(user_pubkey) = Pubkey::from_str(&query_params.pubkey) {
            let res = rr_database.get_miner_contributions(user_pubkey.to_string()).await;

            match res {
                Ok(contributions) => Ok(Json(contributions)),
                Err(_) => Err("Failed to get contributions for miner".to_string()),
            }
        } else {
            Err("Invalid public key".to_string())
        }
    } else {
        return Err("Stats not enabled for this server.".to_string());
    }
}

#[derive(Deserialize)]
struct GetLastClaimParams {
    pubkey: String,
}

async fn get_miner_last_claim(
    query_params: Query<GetLastClaimParams>,
    Extension(rr_database): Extension<Arc<RrDatabase>>,
    Extension(mine_config): Extension<Arc<MineConfig>>,
) -> Result<Json<LastClaim>, String> {
    if mine_config.stats_enabled {
        if let Ok(user_pubkey) = Pubkey::from_str(&query_params.pubkey) {
            let res = rr_database.get_last_claim_by_pubkey(user_pubkey.to_string()).await;

            match res {
                Ok(last_claim) => Ok(Json(last_claim)),
                Err(_) => Err("Failed to get last claim for miner".to_string()),
            }
        } else {
            Err("Invalid public key".to_string())
        }
    } else {
        return Err("Stats not enabled for this server.".to_string());
    }
}

async fn get_miner_balance(
    query_params: Query<PubkeyParam>,
    Extension(rpc_client): Extension<Arc<RpcClient>>,
) -> impl IntoResponse {
    if let Ok(user_pubkey) = Pubkey::from_str(&query_params.pubkey) {
        let miner_token_account = get_associated_token_address(&user_pubkey, &get_ore_mint());
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
            .status(StatusCode::BAD_REQUEST)
            .body("Invalid public key".to_string())
            .unwrap();
    }
}

// MI: legacy stake multiplier is sunset
// it's now a constant (1.0)
async fn get_stake_multiplier(
    Extension(_rpc_client): Extension<Arc<RpcClient>>,
    Extension(mine_config): Extension<Arc<MineConfig>>,
) -> impl IntoResponse {
    if mine_config.stats_enabled {
        // MI: code before sunset of legacy stake multiplier
        // let proof_pubkey = mini_pool_proof_pubkey(*WALLET_PUBKEY.get().unwrap());
        // let proof = if let Ok(loaded_proof) = get_proof(&rpc_client, proof_pubkey).await {
        //     loaded_proof
        // } else {
        //     error!(target: "server_log", "Failed to load mini pool proof.");
        //     return Err("Failed to load mini pool proof.".to_string());
        // };

        // if let Ok(config) = get_config(&rpc_client).await {
        //     let multiplier = 1.0 + (proof.balance as f64 / config.top_balance as f64).min(1.0f64);
        //     return Ok(Json(multiplier));
        // } else {
        //     return Err("Failed to get ore config account".to_string());
        // }

        let multiplier = 1.0;
        return Ok(Json(multiplier));
    } else {
        return Err("Stats not enabled for this server.".to_string());
    }
}

#[derive(Deserialize)]
struct ConnectedMinersParams {
    pubkey: Option<String>,
}

async fn get_connected_miners(
    query_params: Query<ConnectedMinersParams>,
    State(app_state): State<Arc<RwLock<AppState>>>,
) -> impl IntoResponse {
    let reader = app_state.read().await;
    let socks = reader.sockets.clone();
    drop(reader);

    if let Some(pubkey_str) = &query_params.pubkey {
        if let Ok(user_pubkey) = Pubkey::from_str(&pubkey_str) {
            let mut connection_count = 0;

            for (_addr, client_connection) in socks.iter() {
                if user_pubkey.eq(&client_connection.pubkey) {
                    connection_count += 1;
                }
            }

            return Response::builder()
                .status(StatusCode::OK)
                .body(connection_count.to_string())
                .unwrap();
        } else {
            error!(target: "server_log", "Get connected miners with invalid pubkey");
            return Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .body("Invalid Pubkey".to_string())
                .unwrap();
        }
    } else {
        return Response::builder().status(StatusCode::OK).body(socks.len().to_string()).unwrap();
    }
}

async fn get_timestamp() -> impl IntoResponse {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("Time went backwards").as_secs();
    return Response::builder().status(StatusCode::OK).body(now.to_string()).unwrap();
}

#[derive(Deserialize)]
struct ClaimParams {
    timestamp: u64,
    receiver_pubkey: String,
    amount: u64,
}

async fn post_claim(
    TypedHeader(auth_header): TypedHeader<axum_extra::headers::Authorization<Basic>>,
    Extension(rpc_client): Extension<Arc<RpcClient>>,
    Extension(database): Extension<Arc<Database>>,
    Extension(claims_queue): Extension<Arc<ClaimsQueue>>,
    query_params: Query<ClaimParams>,
) -> impl IntoResponse {
    let msg_timestamp = query_params.timestamp;

    let miner_pubkey_str = auth_header.username();
    let signed_msg = auth_header.password();

    let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("Time went backwards").as_secs();

    // Signed authentication message is only valid for 30 seconds
    if (now - msg_timestamp) >= 30 {
        return Err((StatusCode::UNAUTHORIZED, "Timestamp too old.".to_string()));
    }
    let receiver_pubkey = match Pubkey::from_str(&query_params.receiver_pubkey) {
        Ok(pubkey) => pubkey,
        Err(_) => {
            return Err((StatusCode::BAD_REQUEST, "Invalid receiver_pubkey provided.".to_string()))
        },
    };

    if let Ok(miner_pubkey) = Pubkey::from_str(miner_pubkey_str) {
        if let Ok(signature) = Signature::from_str(signed_msg) {
            let amount = query_params.amount;
            let mut signed_msg = vec![];
            signed_msg.extend(msg_timestamp.to_le_bytes());
            signed_msg.extend(receiver_pubkey.to_bytes());
            signed_msg.extend(amount.to_le_bytes());

            if signature.verify(&miner_pubkey.to_bytes(), &signed_msg) {
                let reader = claims_queue.queue.read().await;
                let queue = reader.clone();
                drop(reader);

                if queue.contains_key(&miner_pubkey) {
                    return Err((StatusCode::TOO_MANY_REQUESTS, "QUEUED".to_string()));
                }

                let amount = query_params.amount;

                // 0.00500000000 ORE
                let ore_mint = get_ore_mint();
                let receiver_token_account =
                    get_associated_token_address(&receiver_pubkey, &ore_mint);
                let mut is_creating_ata = true;
                if let Ok(response) =
                    rpc_client.get_token_account_balance(&receiver_token_account).await
                {
                    if let Some(_amount) = response.ui_amount {
                        info!(target: "server_log", "Claim recipient already has a valid token account.");
                        is_creating_ata = false;
                    }
                }

                // if amount < 5_000_000 (0.00005000000 ORE)
                if amount < MIN_CLAIM_AMOUNT_EXISTS_ATA {
                    let min_claim_amount_dec: f64 = (MIN_CLAIM_AMOUNT_EXISTS_ATA as f64)
                        / 10f64.powf(ORE_TOKEN_DECIMALS as f64);
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!("The claim minimum is {min_claim_amount_dec} ORE even if you have alreay a valid token account."),
                    ));
                }

                // if amount < 500_000_000 (0.005000000 ORE)
                if is_creating_ata && amount < MIN_CLAIM_AMOUNT_NOT_EXISTS_ATA {
                    let min_claim_amount_dec: f64 = (MIN_CLAIM_AMOUNT_NOT_EXISTS_ATA as f64)
                        / 10f64.powf(ORE_TOKEN_DECIMALS as f64);
                    return Err((
                        StatusCode::BAD_REQUEST,
                        format!("Since no valid token account already exists, the claim minimum is {min_claim_amount_dec} ORE"),
                    ));
                }

                if let Ok(miner_rewards) =
                    database.get_miner_rewards(miner_pubkey.to_string()).await
                {
                    if amount > miner_rewards.balance as u64 {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            "claim amount exceeds miner rewards balance.".to_string(),
                        ));
                    }

                    if let Ok(last_claim) = database.get_last_claim(miner_rewards.miner_id).await {
                        let last_claim_ts = last_claim.created.and_utc().timestamp();
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .expect("Time went backwards")
                            .as_secs() as i64;
                        let time_difference = now - last_claim_ts;
                        if time_difference <= 1800 {
                            return Err((
                                StatusCode::TOO_MANY_REQUESTS,
                                time_difference.to_string(),
                            ));
                        }
                    }

                    let mut writer = claims_queue.queue.write().await;
                    writer.insert(miner_pubkey, ClaimsQueueItem { receiver_pubkey, amount });
                    drop(writer);
                    return Ok((StatusCode::OK, "SUCCESS"));
                } else {
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "failed to get miner account from database".to_string(),
                    ));
                }
            } else {
                return Err((StatusCode::UNAUTHORIZED, "Sig verification failed".to_string()));
            }
        } else {
            return Err((StatusCode::UNAUTHORIZED, "Invalid signature".to_string()));
        }
    } else {
        error!(target: "server_log", "Claim with invalid pubkey");
        return Err((StatusCode::BAD_REQUEST, "Invalid Pubkey".to_string()));
    }
}

#[derive(Deserialize)]
struct WsQueryParams {
    timestamp: u64,
}

#[debug_handler]
async fn ws_handler(
    ws: WebSocketUpgrade,
    TypedHeader(auth_header): TypedHeader<axum_extra::headers::Authorization<Basic>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(app_state): State<Arc<RwLock<AppState>>>,
    Extension(client_channel): Extension<UnboundedSender<ClientMessage>>,
    Extension(database): Extension<Arc<Database>>,
    query_params: Query<WsQueryParams>,
) -> impl IntoResponse {
    let msg_timestamp = query_params.timestamp;

    let pubkey = auth_header.username();
    let signed_msg = auth_header.password();

    let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("Time went backwards").as_secs();

    // Signed authentication message is only valid for 30 seconds
    if (now - query_params.timestamp) >= 30 {
        return Err((StatusCode::UNAUTHORIZED, "Timestamp too old."));
    }

    let powered_by_dbms = POWERED_BY_DBMS.get_or_init(|| {
        let key = "POWERED_BY_DBMS";
        match std::env::var(key) {
            Ok(val) => {
                PoweredByDbms::from_str(&val).expect("POWERED_BY_DBMS must be set correctly.")
            },
            Err(_) => PoweredByDbms::Unavailable,
        }
    });

    let pool_operator_wallet_pubkey = WALLET_PUBKEY.get().unwrap();

    // verify client
    if let Ok(user_pubkey) = Pubkey::from_str(pubkey) {
        if powered_by_dbms == &PoweredByDbms::Postgres || powered_by_dbms == &PoweredByDbms::Sqlite
        {
            info!(target: "server_log", "Check if the miner record exists in the database");
            let db_miner = database.get_miner_by_pubkey_str(pubkey.to_string()).await;

            let miner;
            match db_miner {
                Ok(db_miner) => {
                    miner = db_miner;
                },
                Err(DatabaseError::QueryFailed) => {
                    info!(target: "server_log", "Miner pubkey record missing from database. Inserting...");
                    let add_miner_result = database
                        .add_new_miner(user_pubkey.to_string(), true, "Enrolled".to_string())
                        .await;
                    miner =
                        database.get_miner_by_pubkey_str(user_pubkey.to_string()).await.unwrap();

                    let wallet_pubkey = *pool_operator_wallet_pubkey; // MI, all clients share operator mini pool(== solo pool)

                    let db_pool =
                        database.get_pool_by_authority_pubkey(wallet_pubkey.to_string()).await;

                    let pool;
                    let mut add_pool_result = Ok::<(), DatabaseError>(());
                    match db_pool {
                        Ok(db_pool) => {
                            pool = db_pool;
                        },
                        Err(DatabaseError::QueryFailed) => {
                            info!(target: "server_log", "Pool record missing from database. Inserting...");
                            let proof_pubkey = utils::mini_pool_proof_pubkey(wallet_pubkey);
                            let pool_pubkey = utils::mini_pool_pubkey(wallet_pubkey);
                            add_pool_result = database
                                .add_new_pool(
                                    wallet_pubkey.to_string(),
                                    proof_pubkey.to_string(),
                                    pool_pubkey.to_string(),
                                )
                                .await;
                            pool = database
                                .get_pool_by_authority_pubkey(wallet_pubkey.to_string())
                                .await
                                .unwrap();
                        },
                        Err(DatabaseError::FailedToGetConnectionFromPool) => {
                            error!(target: "server_log", "Failed to get database pool connection.");
                            return Err((
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "Internal Server Error",
                            ));
                        },
                        Err(_) => {
                            error!(target: "server_log", "DB Error: Catch all.");
                            return Err((
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "Internal Server Error",
                            ));
                        },
                    }

                    if add_miner_result.is_ok() && add_pool_result.is_ok() {
                        let new_reward = InsertReward { miner_id: miner.id, pool_id: pool.id };
                        let result = database.add_new_reward(new_reward).await;

                        if result.is_ok() {
                            info!(target: "server_log", "Miner and rewards tracker added to database");
                        } else {
                            error!(target: "server_log", "Failed to add miner rewards tracker to database");
                            return Err((
                                StatusCode::UNAUTHORIZED,
                                "Failed to add miner rewards tracker to database",
                            ));
                        }
                        info!(target: "server_log", "Miner record added to database");
                    } else {
                        error!(target: "server_log", "Failed to add miner record to database");
                        return Err((
                            StatusCode::UNAUTHORIZED,
                            "Failed to add miner record to database",
                        ));
                    }
                },
                Err(DatabaseError::FailedToGetConnectionFromPool) => {
                    error!(target: "server_log", "Failed to get database pool connection.");
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error"));
                },
                Err(_) => {
                    error!(target: "server_log", "DB Error: Catch all.");
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error"));
                },
            }

            if !miner.enabled {
                return Err((StatusCode::UNAUTHORIZED, "pubkey is not authorized to mine"));
            }

            if let Ok(signature) = Signature::from_str(signed_msg) {
                let ts_msg = msg_timestamp.to_le_bytes();

                if signature.verify(&user_pubkey.to_bytes(), &ts_msg) {
                    info!(target: "server_log", "Client: {addr} connected with pubkey {pubkey}.");
                    return Ok(ws.on_upgrade(move |socket| {
                        handle_socket(
                            socket,
                            addr,
                            user_pubkey,
                            miner.id,
                            ClientVersion::V1,
                            app_state,
                            client_channel,
                        )
                    }));
                } else {
                    return Err((StatusCode::UNAUTHORIZED, "Sig verification failed"));
                }
            } else {
                return Err((StatusCode::UNAUTHORIZED, "Invalid signature"));
            }
        } else {
            // NO DBMS FOUND
            warn!(target: "server_log", "WARNING: NO DBMS FOUND. NO CLAIM FOR REMOTE MINERS!");
            eprintln!("WARNING: NO DBMS FOUND. NO CLAIM FOR REMOTE MINERS!");

            // MI: comment out to allow multiple miners to use the same wallet.
            // {
            //     let mut already_connected = false;
            //     for (_, client_connection) in app_state.read().await.sockets.iter() {
            //         if user_pubkey == client_connection.pubkey {
            //             already_connected = true;
            //             break;
            //         }
            //     }
            //     if already_connected {
            //         return Err((
            //             StatusCode::TOO_MANY_REQUESTS,
            //             "A client is already connected with that wallet",
            //         ));
            //     }
            // };

            if let Ok(signature) = Signature::from_str(signed_msg) {
                let ts_msg = msg_timestamp.to_le_bytes();

                if signature.verify(&user_pubkey.to_bytes(), &ts_msg) {
                    info!(target: "server_log", "Client: {addr} connected with pubkey {pubkey}.");
                    return Ok(ws.on_upgrade(move |socket| {
                        handle_socket(
                            socket,
                            addr,
                            user_pubkey,
                            // MI: default miner_id for non-dbms
                            i64::MAX,
                            ClientVersion::V1,
                            app_state,
                            client_channel,
                        )
                    }));
                } else {
                    return Err((StatusCode::UNAUTHORIZED, "Sig verification failed"));
                }
            } else {
                return Err((StatusCode::UNAUTHORIZED, "Invalid signature"));
            }
        }
    } else {
        return Err((StatusCode::UNAUTHORIZED, "Invalid pubkey"));
    }
}

async fn handle_socket(
    mut socket: WebSocket,
    who: SocketAddr,
    who_pubkey: Pubkey,
    who_miner_id: i64,
    client_version: ClientVersion,
    rw_app_state: Arc<RwLock<AppState>>,
    client_channel: UnboundedSender<ClientMessage>,
) {
    if socket.send(axum::extract::ws::Message::Ping(vec![1, 2, 3])).await.is_ok() {
        debug!(target: "server_log", "Pinged {who}... pubkey: {who_pubkey}");
    } else {
        error!(target: "server_log", "could not ping {who} pubkey: {who_pubkey}");

        // if we can't ping we can't do anything, return to close the connection
        return;
    }

    let (sender, mut receiver) = socket.split();
    let mut app_state = rw_app_state.write().await;
    if app_state.sockets.contains_key(&who) {
        info!(target: "server_log", "Socket addr: {who} already has an active connection");
        return;
    } else {
        let new_client_connection = ClientConnection {
            pubkey: who_pubkey,
            miner_id: who_miner_id,
            client_version,
            socket: Arc::new(Mutex::new(sender)),
        };
        app_state.sockets.insert(who, new_client_connection);
    }
    drop(app_state);

    let _ = tokio::spawn(async move {
        // // MI: vanilla. by design while let will exit when None received
        // while let Some(Ok(msg)) = receiver.next().await {
        //     if process_message(msg, who, client_channel.clone()).is_break() {
        //         break;
        //     }
        // }

        // MI: use loop for else processing, since by design while let will exit when None received
        loop {
            if let Some(Ok(msg)) = receiver.next().await {
                if process_message(msg, who, client_channel.clone()).is_break() {
                    break;
                }
            } else {
                // receiver got None, the stream ended.
                // None is returned when the sender half has dropped, indicating that no further values can be received.
                warn!(target: "server_log", "The sender half of websocket has been dropped. No more messages will be received from {who}. Exit the loop.");
                break;
            }
        }
    })
    .await;

    let mut app_state = rw_app_state.write().await;
    app_state.sockets.remove(&who);
    drop(app_state);

    info!(target: "server_log", "Client: {} disconnected!", who_pubkey.to_string());
}

fn process_message(
    msg: Message,
    who: SocketAddr,
    client_channel: UnboundedSender<ClientMessage>,
) -> ControlFlow<(), ()> {
    match msg {
        Message::Text(_t) => {
            // info!(target: "server_log", ">>> {who} sent str: {t:?}");
        },
        Message::Binary(d) => {
            // first 8 bytes are message type
            let message_type = d[0];
            match message_type {
                0 => {
                    let msg = ClientMessage::Ready(who);
                    let _ = client_channel.send(msg);
                },
                1 => {
                    let msg = ClientMessage::Mining(who);
                    let _ = client_channel.send(msg);
                },
                2 => {
                    // parse solution from message data
                    let mut solution_bytes = [0u8; 16];
                    // extract (16 u8's) from data for hash digest
                    let mut b_index = 1;
                    for i in 0..16 {
                        solution_bytes[i] = d[i + b_index];
                    }
                    b_index += 16;

                    // extract 64 bytes (8 u8's)
                    let mut nonce = [0u8; 8];
                    for i in 0..8 {
                        nonce[i] = d[i + b_index];
                    }
                    b_index += 8;

                    let mut pubkey = [0u8; 32];
                    for i in 0..32 {
                        pubkey[i] = d[i + b_index];
                    }

                    b_index += 32;

                    let signature_bytes = d[b_index..].to_vec();
                    if let Ok(sig_str) = String::from_utf8(signature_bytes.clone()) {
                        if let Ok(sig) = Signature::from_str(&sig_str) {
                            let pubkey = Pubkey::new_from_array(pubkey);

                            let mut hash_nonce_message = [0; 24];
                            hash_nonce_message[0..16].copy_from_slice(&solution_bytes);
                            hash_nonce_message[16..24].copy_from_slice(&nonce);

                            if sig.verify(&pubkey.to_bytes(), &hash_nonce_message) {
                                let solution = Solution::new(solution_bytes, nonce);

                                let msg = ClientMessage::BestSolution(who, solution, pubkey);
                                let _ = client_channel.send(msg);
                            } else {
                                error!(target: "server_log", "Client contribution sig verification failed.");
                            }
                        } else {
                            error!(target: "server_log", "Failed to parse into Signature.");
                        }
                    } else {
                        error!(target: "server_log", "Failed to parse signed message from client.");
                    }
                },
                _ => {
                    error!(target: "server_log", ">>> {} sent an invalid message", who);
                },
            }
        },
        Message::Close(c) => {
            if let Some(cf) = c {
                info!(target: "server_log", ">>> {} sent close with code {} and reason `{}`", who, cf.code, cf.reason);
            } else {
                info!(target: "server_log", ">>> {who} somehow sent close message without CloseFrame");
            }
            return ControlFlow::Break(());
        },
        Message::Pong(_v) => {
            let msg = ClientMessage::Pong(who);
            let _ = client_channel.send(msg);
        },
        Message::Ping(_v) => {
            //info!(target: "server_log", ">>> {who} sent ping with {v:?}");
        },
    }

    ControlFlow::Continue(())
}

fn styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Red.on_default() | Effects::BOLD)
        .usage(AnsiColor::Red.on_default() | Effects::BOLD)
        .literal(AnsiColor::Blue.on_default() | Effects::BOLD)
        .placeholder(AnsiColor::Green.on_default())
}
