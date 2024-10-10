use {
    crate::database::{Database, PoolSubmissionResult},
    base64::prelude::*,
    clap::{arg, Parser},
    drillx::equix,
    futures_util::{stream::SplitSink, SinkExt, StreamExt},
    indicatif::{ProgressBar, ProgressStyle},
    solana_sdk::{signature::Keypair, signer::Signer},
    spl_token::amount_to_ui_amount,
    std::{
        env,
        mem::size_of,
        ops::{ControlFlow, Range},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    },
    tokio::{
        net::TcpStream,
        sync::{
            mpsc::{UnboundedReceiver, UnboundedSender},
            Mutex,
        },
        time::timeout,
    },
    tokio_tungstenite::{
        connect_async,
        tungstenite::{
            handshake::client::{generate_key, Request},
            Message,
        },
        MaybeTlsStream, WebSocketStream,
    },
};

#[derive(Debug)]
pub struct ServerMessagePoolSubmissionResult {
    difficulty: u32,
    total_balance: f64,
    total_rewards: f64,
    top_stake: f64,
    multiplier: f64,
    active_miners: u32,
    challenge: [u8; 32],
    best_nonce: u64,
    miner_supplied_difficulty: u32,
    miner_earned_rewards: f64,
    miner_percentage: f64,
}

impl ServerMessagePoolSubmissionResult {
    pub fn new_from_bytes(b: Vec<u8>) -> Self {
        let mut b_index = 1;

        let data_size = size_of::<u32>();
        let mut data_bytes = [0u8; size_of::<u32>()];
        for i in 0..data_size {
            data_bytes[i] = b[i + b_index];
        }
        b_index += data_size;
        let difficulty = u32::from_le_bytes(data_bytes);

        let data_size = size_of::<f64>();
        let mut data_bytes = [0u8; size_of::<f64>()];
        for i in 0..data_size {
            data_bytes[i] = b[i + b_index];
        }
        b_index += data_size;
        let total_balance = f64::from_le_bytes(data_bytes);

        let data_size = size_of::<f64>();
        let mut data_bytes = [0u8; size_of::<f64>()];
        for i in 0..data_size {
            data_bytes[i] = b[i + b_index];
        }
        b_index += data_size;
        let total_rewards = f64::from_le_bytes(data_bytes);

        let data_size = size_of::<f64>();
        let mut data_bytes = [0u8; size_of::<f64>()];
        for i in 0..data_size {
            data_bytes[i] = b[i + b_index];
        }
        b_index += data_size;
        let top_stake = f64::from_le_bytes(data_bytes);

        let data_size = size_of::<f64>();
        let mut data_bytes = [0u8; size_of::<f64>()];
        for i in 0..data_size {
            data_bytes[i] = b[i + b_index];
        }
        b_index += data_size;
        let multiplier = f64::from_le_bytes(data_bytes);

        let data_size = size_of::<u32>();
        let mut data_bytes = [0u8; size_of::<u32>()];
        for i in 0..data_size {
            data_bytes[i] = b[i + b_index];
        }
        b_index += data_size;
        let active_miners = u32::from_le_bytes(data_bytes);

        let data_size = 32;
        let mut data_bytes = [0u8; 32];
        for i in 0..data_size {
            data_bytes[i] = b[i + b_index];
        }
        b_index += data_size;
        let challenge = data_bytes.clone();

        let data_size = size_of::<u64>();
        let mut data_bytes = [0u8; size_of::<u64>()];
        for i in 0..data_size {
            data_bytes[i] = b[i + b_index];
        }
        b_index += data_size;
        let best_nonce = u64::from_le_bytes(data_bytes);

        let data_size = size_of::<u32>();
        let mut data_bytes = [0u8; size_of::<u32>()];
        for i in 0..data_size {
            data_bytes[i] = b[i + b_index];
        }
        b_index += data_size;
        let miner_supplied_difficulty = u32::from_le_bytes(data_bytes);

        let data_size = size_of::<f64>();
        let mut data_bytes = [0u8; size_of::<f64>()];
        for i in 0..data_size {
            data_bytes[i] = b[i + b_index];
        }
        b_index += data_size;
        let miner_earned_rewards = f64::from_le_bytes(data_bytes);

        let data_size = size_of::<f64>();
        let mut data_bytes = [0u8; size_of::<f64>()];
        for i in 0..data_size {
            data_bytes[i] = b[i + b_index];
        }
        //b_index += data_size;
        let miner_percentage = f64::from_le_bytes(data_bytes);

        ServerMessagePoolSubmissionResult {
            difficulty,
            total_balance,
            total_rewards,
            top_stake,
            multiplier,
            active_miners,
            challenge,
            best_nonce,
            miner_supplied_difficulty,
            miner_earned_rewards,
            miner_percentage,
        }
    }

    pub fn _to_message_binary(&self) -> Vec<u8> {
        let mut bin_data = Vec::new();
        bin_data.push(1u8);
        bin_data.extend_from_slice(&self.difficulty.to_le_bytes());
        bin_data.extend_from_slice(&self.total_balance.to_le_bytes());
        bin_data.extend_from_slice(&self.total_rewards.to_le_bytes());
        bin_data.extend_from_slice(&self.top_stake.to_le_bytes());
        bin_data.extend_from_slice(&self.multiplier.to_le_bytes());
        bin_data.extend_from_slice(&self.active_miners.to_le_bytes());
        bin_data.extend_from_slice(&self.challenge);
        bin_data.extend_from_slice(&self.best_nonce.to_le_bytes());
        bin_data.extend_from_slice(&self.miner_supplied_difficulty.to_le_bytes());
        bin_data.extend_from_slice(&self.miner_earned_rewards.to_le_bytes());
        bin_data.extend_from_slice(&self.miner_percentage.to_le_bytes());

        bin_data
    }
}

#[derive(Debug)]
pub enum ServerMessage {
    StartMining([u8; 32], Range<u64>, u64),
    PoolSubmissionResult(ServerMessagePoolSubmissionResult),
}

#[derive(Debug, Clone, Copy)]
pub struct ThreadSubmission {
    nonce: u64,
    difficulty: u32,
    pub d: [u8; 16], // digest
}

#[derive(Debug, Clone, Copy)]
pub enum MessageSubmissionProcessor {
    Submission(ThreadSubmission),
    Reset,
    Finish,
}

#[derive(Debug, Parser)]
pub struct MineArgs {
    #[arg(
        long,
        value_name = "threads",
        default_value = "4",
        help = "Number of threads to use while mining"
    )]
    pub threads: u32,
    #[arg(
        long,
        value_name = "BUFFER",
        default_value = "0",
        help = "Buffer time in seconds, to send the submission to the server earlier"
    )]
    pub buffer: u32,
}

const MIN_DIFF: u32 = 8; // MI, align with server

pub async fn mine(args: MineArgs, key: Keypair, url: String, unsecure: bool) {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    let key = Arc::new(key);

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        let base_url = url.clone();
        // MI: lan addr is allowed, example: 172.xx.xx.xxx:3000
        let mut ws_url_str =
            if unsecure { format!("ws://{}/v1/ws", url) } else { format!("wss://{}/v1/ws", url) };

        let client = reqwest::Client::new();

        let http_prefix = if unsecure { "http".to_string() } else { "https".to_string() };

        let timestamp = match client
            .get(format!("{}://{}/timestamp", http_prefix, base_url))
            .send()
            .await
        {
            Ok(res) => {
                if res.status().as_u16() >= 200 && res.status().as_u16() < 300 {
                    if let Ok(ts) = res.text().await {
                        if let Ok(ts) = ts.parse::<u64>() {
                            ts
                        } else {
                            println!("Server response body for /timestamp failed to parse, contact admin.");
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            continue;
                        }
                    } else {
                        println!("Server response body for /timestamp is empty, contact admin.");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                } else {
                    println!("Failed to get timestamp from server. StatusCode: {}", res.status());
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            },
            Err(e) => {
                println!("Failed to get timestamp from server.\nError: {}", e);
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            },
        };

        println!("Server Timestamp: {}", timestamp);

        let ts_msg = timestamp.to_le_bytes();
        let sig = key.sign_message(&ts_msg);

        ws_url_str.push_str(&format!("?timestamp={}", timestamp));
        let url = url::Url::parse(&ws_url_str).expect("Failed to parse server url");
        let host = url.host_str().expect("Invalid host in server url");
        let mut threads = args.threads;

        let auth = BASE64_STANDARD.encode(format!("{}:{}", key.pubkey(), sig));

        println!("Connecting to server...");
        let request = Request::builder()
            .method("GET")
            .uri(url.to_string())
            .header("Sec-Websocket-Key", generate_key())
            .header("Host", host)
            .header("Upgrade", "websocket")
            .header("Connection", "upgrade")
            .header("Sec-Websocket-Version", "13")
            .header("Authorization", format!("Basic {}", auth))
            .body(())
            .unwrap();

        match connect_async(request).await {
            Ok((ws_stream, _)) => {
                println!("Connected to network!");

                let (sender, mut receiver) = ws_stream.split();
                let (message_sender, mut message_receiver) =
                    tokio::sync::mpsc::unbounded_channel::<ServerMessage>();

                // let receiver_thread = tokio::spawn(async move {
                //     loop {
                //         if let Some(Ok(message)) = receiver.next().await {
                //             if process_message(message, message_sender.clone()).is_break() {
                //                 break;
                //             }
                //         } else {
                //             // receiver got None, the stream ended.
                //             // None is returned when the sender half has dropped, indicating that
                // no further values can be received.             eprintln!("The
                // sender half has been dropped. No more messages will be received. Exit the
                // loop.");             break; // exit loop
                //         }
                //     }
                // });

                let (solution_processor_message_sender, solution_processor_message_receiver) =
                    tokio::sync::mpsc::unbounded_channel::<MessageSubmissionProcessor>();

                let sender = Arc::new(Mutex::new(sender));
                let app_key = key.clone();
                let app_socket_sender = sender.clone();
                tokio::spawn(async move {
                    submission_processor(
                        app_key,
                        solution_processor_message_receiver,
                        app_socket_sender,
                    )
                    .await;
                });

                let solution_processor_submission_sender =
                    Arc::new(solution_processor_message_sender);

                let msend = message_sender.clone();
                let processor_submission_sender = solution_processor_submission_sender.clone();
                let receiver_thread = tokio::spawn(async move {
                    let mut last_start_mine_instant = Instant::now();
                    loop {
                        match timeout(Duration::from_secs(45), receiver.next()).await {
                            Ok(Some(Ok(message))) => {
                                match process_message(message, msend.clone()) {
                                    ControlFlow::Break(_) => {
                                        break;
                                    },
                                    ControlFlow::Continue(got_start_mining) => {
                                        if got_start_mining {
                                            last_start_mine_instant = Instant::now();
                                        }
                                    },
                                }

                                if last_start_mine_instant.elapsed().as_secs() >= 120 {
                                    eprintln!("Last start mining message was over 2 minutes ago. Closing websocket for reconnection.");
                                    break;
                                }
                            },
                            Ok(Some(Err(e))) => {
                                eprintln!("Websocket error: {}", e);
                                break;
                            },
                            Ok(None) => {
                                eprintln!("Websocket closed gracefully");
                                break;
                            },
                            Err(_) => {
                                eprintln!("Websocket receiver timeout, assuming disconnection");
                                break;
                            },
                        }
                    }

                    println!("Websocket receiver closed or timed out.");
                    println!("Cleaning up channels...");
                    let _ = processor_submission_sender.send(MessageSubmissionProcessor::Finish);
                    drop(msend);
                    drop(message_sender);
                });

                // send Ready message
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Time went backwards")
                    .as_secs();

                let msg = now.to_le_bytes();
                let sig = key.sign_message(&msg).to_string().as_bytes().to_vec();
                let mut bin_data: Vec<u8> = Vec::new();
                bin_data.push(0u8);
                bin_data.extend_from_slice(&key.pubkey().to_bytes());
                bin_data.extend_from_slice(&msg);
                bin_data.extend(sig);

                let mut lock = sender.lock().await;
                let _ = lock.send(Message::Binary(bin_data)).await;
                drop(lock);

                let (db_sender, mut db_receiver) =
                    tokio::sync::mpsc::unbounded_channel::<PoolSubmissionResult>();

                tokio::spawn(async move {
                    let app_db = Database::new();

                    while let Some(msg) = db_receiver.recv().await {
                        app_db.add_new_pool_submission(msg);
                        let total_earnings = amount_to_ui_amount(
                            app_db.get_todays_earnings(),
                            ore_api::consts::TOKEN_DECIMALS,
                        );
                        println!("Todays Earnings: {} ORE\n", total_earnings);
                    }
                });

                // receive messages
                let s_processor_submission_sender = solution_processor_submission_sender.clone();
                while let Some(msg) = message_receiver.recv().await {
                    let processor_submission_sender = s_processor_submission_sender.clone();
                    let db_sender = db_sender.clone();
                    tokio::spawn({
                        let message_sender = sender.clone();
                        let key = key.clone();
                        let running = running.clone();
                        async move {
                            if !running.load(Ordering::SeqCst) {
                                return;
                            }

                            match msg {
                                ServerMessage::StartMining(challenge, nonce_range, cutoff) => {
                                    println!(
                                        "\nMission received. New Challenge: {}",
                                        BASE64_STANDARD.encode(challenge)
                                    );
                                    println!(
                                        "Nonce range: {} - {}",
                                        nonce_range.start, nonce_range.end
                                    );
                                    println!("Start mining... will cutoff in: {}s", cutoff);

                                    // Adjust the cutoff with the buffer
                                    let cutoff = cutoff.saturating_sub(args.buffer as u64);
                                    // if cutoff > 60 {
                                    //     cutoff = 55;
                                    // }

                                    // Detect if running on Windows and set symbols accordingly
                                    let pb = if env::consts::OS == "windows" {
                                        ProgressBar::new_spinner().with_style(
                                            ProgressStyle::default_spinner()
                                                .tick_strings(&["-", "\\", "|", "/"]) // Use simple ASCII symbols
                                                .template("{spinner:.green} {msg}")
                                                .expect("Failed to set progress bar template"),
                                        )
                                    } else {
                                        ProgressBar::new_spinner().with_style(
                                            ProgressStyle::default_spinner()
                                                .tick_strings(&[
                                                    "⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇",
                                                    "⠏",
                                                ])
                                                .template("{spinner:.red} {msg}")
                                                .expect("Failed to set progress bar template"),
                                        )
                                    };

                                    println!();
                                    pb.set_message("Mining...");
                                    pb.enable_steady_tick(Duration::from_millis(120));

                                    // Original mining code
                                    let hash_timer = Instant::now();
                                    let core_ids = core_affinity::get_core_ids().unwrap();
                                    // let max_cores =
                                    // std::thread::available_parallelism().unwrap().get();
                                    let max_cores = core_ids.len();
                                    if threads > max_cores as u32 {
                                        println!(
                                            "Arg --threads {} exceeds available cores({}), the exceeding part will be ignored.",
                                            threads, max_cores
                                        );

                                        threads = max_cores as u32
                                    }
                                    let nonces_per_thread = 10_000;
                                    let handles = core_ids
                                        .into_iter()
                                        .map(|i| {
                                            let running = running.clone(); // Capture running in thread
                                            let processor_submission_sender =
                                                processor_submission_sender.clone();
                                            std::thread::spawn({
                                                let mut memory = equix::SolverMemory::new();
                                                move || {
                                                    if (i.id as u32).ge(&threads) {
                                                        return None;
                                                    }

                                                    let _ = core_affinity::set_for_current(i);

                                                    let first_nonce = nonce_range.start
                                                        + (nonces_per_thread * (i.id as u64));
                                                    let mut nonce = first_nonce;
                                                    let mut best_nonce = nonce;
                                                    let mut best_difficulty = 0;
                                                    let mut best_hash = drillx::Hash::default();
                                                    let mut total_hashes: u64 = 0;

                                                    'challenge: loop {
                                                        // Check if Ctrl+C was pressed
                                                        if !running.load(Ordering::SeqCst) {
                                                            return None;
                                                        }

                                                        // Create hash
                                                        for hx in drillx::hashes_with_memory(
                                                            &mut memory,
                                                            &challenge,
                                                            &nonce.to_le_bytes(),
                                                        ) {
                                                            total_hashes += 1;
                                                            let difficulty = hx.difficulty();
                                                            if difficulty.gt(&MIN_DIFF) && difficulty.gt(&best_difficulty) {
                                                                // MI: solution-level, too much submissions
                                                                let thread_submission = ThreadSubmission {
                                                                    nonce,
                                                                    difficulty,
                                                                    d: hx.d,
                                                                };
                                                                if let Err(_) = processor_submission_sender.send(
                                                                    MessageSubmissionProcessor::Submission(
                                                                        thread_submission,
                                                                    ),
                                                                ) {
                                                                    eprintln!("Failed to send found hash to internal submission processor");
                                                                }
                                                                best_nonce = nonce;
                                                                best_difficulty = difficulty;
                                                                best_hash = hx;
                                                            }
                                                        }

                                                        // Exit if processed nonce range
                                                        if nonce >= nonce_range.end {
                                                            break 'challenge;
                                                        }

                                                        if nonce % 100 == 0 {
                                                            if hash_timer
                                                                .elapsed()
                                                                .as_secs()
                                                                .ge(&cutoff)
                                                            {
                                                                if best_difficulty.ge(&MIN_DIFF) {
                                                                    break 'challenge;
                                                                }
                                                            }
                                                        }

                                                        // Increment nonce
                                                        nonce += 1;
                                                    }

                                                    // // MI: upon cutoff, submit thread-level best solution to server, non-necessary submission
                                                    // let thread_cutoff_submission =
                                                    //     ThreadSubmission {
                                                    //         nonce: best_nonce,
                                                    //         difficulty: best_difficulty,
                                                    //         d: best_hash.d,
                                                    //     };
                                                    // let _ = processor_submission_sender.send(
                                                    //     MessageSubmissionProcessor::Submission(
                                                    //         thread_cutoff_submission,
                                                    //     ),
                                                    // );

                                                    // Return the best nonce
                                                    Some((
                                                        best_nonce,
                                                        best_difficulty,
                                                        best_hash,
                                                        total_hashes,
                                                    ))
                                                }
                                            })
                                        })
                                        .collect::<Vec<_>>();

                                    // Join handles and return best nonce
                                    let mut best_nonce: u64 = 0;
                                    let mut best_difficulty = 0;
                                    let mut best_hash = drillx::Hash::default();
                                    let mut total_nonces_checked = 0;
                                    for h in handles {
                                        if let Ok(Some((nonce, difficulty, hash, nonces_checked))) =
                                            h.join()
                                        {
                                            total_nonces_checked += nonces_checked;
                                            if difficulty > best_difficulty {
                                                best_difficulty = difficulty;
                                                best_nonce = nonce;
                                                best_hash = hash;
                                            }
                                        }
                                    }

                                    // MI: upon cutoff, submit client-level best solution to server,
                                    // which is duplicated with some previous ThreadSubmission.
                                    // Compatible with client version V0
                                    let client_cutoff_submission = ThreadSubmission {
                                        nonce: best_nonce,
                                        difficulty: best_difficulty,
                                        d: best_hash.d,
                                    };
                                    let _ = processor_submission_sender.send(
                                        MessageSubmissionProcessor::Submission(
                                            client_cutoff_submission,
                                        ),
                                    );

                                    let hash_time = hash_timer.elapsed();

                                    // Stop the spinner after mining is done
                                    pb.finish_and_clear();
                                    println!(
                                        "✨ Mission completed! Best diff found: {}",
                                        best_difficulty
                                    );
                                    println!("Processed: {}", total_nonces_checked);
                                    println!("Hash time: {:?}", hash_time);
                                    let hash_time_secs = hash_time.as_secs();
                                    if hash_time_secs.gt(&0) {
                                        println!(
                                            "Hashpower: {:?} H/s",
                                            total_nonces_checked.saturating_div(hash_time_secs)
                                        );
                                    } else {
                                        println!("Hashpower: {:?} H/s", 0);
                                    }

                                    let _ = processor_submission_sender
                                        .send(MessageSubmissionProcessor::Reset);

                                    // Ready up again
                                    let now = SystemTime::now()
                                        .duration_since(UNIX_EPOCH)
                                        .expect("Time went backwards")
                                        .as_secs();

                                    let msg = now.to_le_bytes();
                                    let sig =
                                        key.sign_message(&msg).to_string().as_bytes().to_vec();
                                    let mut bin_data: Vec<u8> = Vec::new();
                                    bin_data.push(0u8);
                                    bin_data.extend_from_slice(&key.pubkey().to_bytes());
                                    bin_data.extend_from_slice(&msg);
                                    bin_data.extend(sig);
                                    {
                                        let mut message_sender = message_sender.lock().await;
                                        if let Err(_) =
                                            message_sender.send(Message::Binary(bin_data)).await
                                        {
                                            let _ = processor_submission_sender
                                                .send(MessageSubmissionProcessor::Finish);
                                            eprintln!("Failed to send Ready message. Returning...");
                                            return;
                                        }
                                    }
                                },
                                ServerMessage::PoolSubmissionResult(data) => {
                                    let pool_earned = (data.total_rewards
                                        * 10f64.powf(ore_api::consts::TOKEN_DECIMALS as f64))
                                        as u64;
                                    let miner_earned = (data.miner_earned_rewards
                                        * 10f64.powf(ore_api::consts::TOKEN_DECIMALS as f64))
                                        as u64;
                                    let ps = PoolSubmissionResult::new(
                                        data.difficulty,
                                        pool_earned,
                                        data.miner_percentage,
                                        data.miner_supplied_difficulty,
                                        miner_earned,
                                    );
                                    let _ = db_sender.send(ps);

                                    let message = format!(
                                        "\n\nChallenge: {}\nPool Submitted Difficulty: {}\nPool Earned:  {:.11} ORE\nPool Balance: {:.11} ORE\nTop Stake:    {:.11} ORE\nPool Multiplier: {:.2}x\n----------------------\nActive Miners: {}\n----------------------\nMiner Submitted Difficulty: {}\nMiner Earned: {:.11} ORE\n{:.2}% of total pool reward\n",
                                        BASE64_STANDARD.encode(data.challenge),
                                        data.difficulty,
                                        data.total_rewards,
                                        data.total_balance,
                                        data.top_stake,
                                        data.multiplier,
                                        data.active_miners,
                                        data.miner_supplied_difficulty,
                                        data.miner_earned_rewards,
                                        data.miner_percentage
                                    );
                                    let _ = data.best_nonce;
                                    println!("{}", message);
                                },
                            }
                        }
                    });
                }

                // If the websocket message receiver finishes, also finish the solution submission
                // sender processor
                let _ = receiver_thread.await;
                let _ =
                    solution_processor_submission_sender.send(MessageSubmissionProcessor::Finish);
                println!("Channels cleaned up, reconnecting...\n");
            },
            Err(e) => {
                match e {
                    tokio_tungstenite::tungstenite::Error::Http(e) => {
                        if let Some(body) = e.body() {
                            println!("Error: {:?}", String::from_utf8(body.to_vec()));
                        } else {
                            println!("Http Error: {:?}", e);
                        }
                    },
                    _ => {
                        println!("Error: {:?}", e);
                    },
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            },
        }
    }
}

fn process_message(
    msg: Message,
    message_channel: UnboundedSender<ServerMessage>,
) -> ControlFlow<(), bool> {
    let mut got_start_mining_message = false;
    match msg {
        Message::Text(t) => {
            println!("{}", t);
        },
        Message::Binary(b) => {
            let message_type = b[0];
            match message_type {
                0 => {
                    if b.len() < 49 {
                        println!("Invalid data for Message StartMining");
                    } else {
                        let mut hash_bytes = [0u8; 32];
                        // extract 256 bytes (32 u8's) from data for hash
                        let mut b_index = 1;
                        for i in 0..32 {
                            hash_bytes[i] = b[i + b_index];
                        }
                        b_index += 32;

                        // extract 64 bytes (8 u8's)
                        let mut cutoff_bytes = [0u8; 8];
                        for i in 0..8 {
                            cutoff_bytes[i] = b[i + b_index];
                        }
                        b_index += 8;
                        let cutoff = u64::from_le_bytes(cutoff_bytes);

                        let mut nonce_start_bytes = [0u8; 8];
                        for i in 0..8 {
                            nonce_start_bytes[i] = b[i + b_index];
                        }
                        b_index += 8;
                        let nonce_start = u64::from_le_bytes(nonce_start_bytes);

                        let mut nonce_end_bytes = [0u8; 8];
                        for i in 0..8 {
                            nonce_end_bytes[i] = b[i + b_index];
                        }
                        let nonce_end = u64::from_le_bytes(nonce_end_bytes);

                        let msg =
                            ServerMessage::StartMining(hash_bytes, nonce_start..nonce_end, cutoff);

                        let _ = message_channel.send(msg);
                        got_start_mining_message = true;
                    }
                },
                1 => {
                    let msg = ServerMessage::PoolSubmissionResult(
                        ServerMessagePoolSubmissionResult::new_from_bytes(b),
                    );
                    let _ = message_channel.send(msg);
                },
                _ => {
                    println!("Failed to parse server message type");
                },
            }
        },
        Message::Ping(_) => {},
        Message::Pong(_) => {},
        Message::Close(v) => {
            println!("Got Close: {:?}", v);
            return ControlFlow::Break(());
        },
        _ => {},
    }

    ControlFlow::Continue(got_start_mining_message)
}

async fn submission_processor(
    key: Arc<Keypair>,
    mut processor_message_receiver: UnboundedReceiver<MessageSubmissionProcessor>,
    socket_sender: Arc<Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>>>,
) {
    let mut best_diff = 0;
    while let Some(msg) = processor_message_receiver.recv().await {
        match msg {
            MessageSubmissionProcessor::Submission(thread_submission) => {
                if thread_submission.difficulty > best_diff {
                    best_diff = thread_submission.difficulty;

                    // Send results to the server
                    let message_type = 2u8; // 1 u8 - BestSolution Message
                    let best_hash_bin = thread_submission.d; // 16 u8
                    let best_nonce_bin = thread_submission.nonce.to_le_bytes(); // 8 u8

                    let mut hash_nonce_message = [0; 24];
                    hash_nonce_message[0..16].copy_from_slice(&best_hash_bin);
                    hash_nonce_message[16..24].copy_from_slice(&best_nonce_bin);
                    let signature =
                        key.sign_message(&hash_nonce_message).to_string().as_bytes().to_vec();

                    let mut bin_data = [0; 57];
                    bin_data[00..1].copy_from_slice(&message_type.to_le_bytes());
                    bin_data[01..17].copy_from_slice(&best_hash_bin);
                    bin_data[17..25].copy_from_slice(&best_nonce_bin);
                    bin_data[25..57].copy_from_slice(&key.pubkey().to_bytes());

                    let mut bin_vec = bin_data.to_vec();
                    bin_vec.extend(signature);

                    let mut message_sender = socket_sender.lock().await;
                    let _ = message_sender.send(Message::Binary(bin_vec)).await;
                    drop(message_sender);
                }
            },
            MessageSubmissionProcessor::Reset => {
                best_diff = 0;

                // Sleep for 2 seconds waiting for next mining mission.
                tokio::time::sleep(Duration::from_secs(2)).await;
            },
            MessageSubmissionProcessor::Finish => {
                return;
            },
        }
    }
}
