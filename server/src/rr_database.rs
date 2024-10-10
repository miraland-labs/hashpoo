#![cfg(feature = "powered-by-dbms-postgres")]
use {
    crate::{
        database::{DatabaseError, PgConfig},
        models::{self, *},
        ChallengeWithDifficulty, Contribution, ContributionWithPubkey,
    },
    deadpool_postgres::{Client, Config, ManagerConfig, Pool, RecyclingMethod, Runtime},
    pg_connection_string::{from_multi_str, ConnectionString},
    rustls::{Certificate, ClientConfig as RustlsClientConfig},
    std::{fs::File, io::BufReader},
    tokio_postgres::NoTls,
    tokio_postgres_rustls::MakeRustlsConnect,
    tracing::debug,
};

pub struct RrDatabase {
    connection_pool: Pool,
}

impl RrDatabase {
    pub fn new(database_uri: Option<impl Into<String>>) -> Self {
        if let Some(connection_string) = database_uri {
            if let Ok([ConnectionString { user, password, hostspecs, database, .. }, ..]) =
                from_multi_str(&connection_string.into(), ",").as_deref()
            {
                let mut deadpool_cfg = Config::new();
                deadpool_cfg.user = user.clone();
                deadpool_cfg.password = password.clone();
                deadpool_cfg.host = Some(hostspecs[0].host.to_string());
                deadpool_cfg.port = hostspecs[0].port;
                deadpool_cfg.dbname = database.clone();
                deadpool_cfg.manager =
                    Some(ManagerConfig { recycling_method: RecyclingMethod::Fast });
                let connection_pool =
                    deadpool_cfg.create_pool(Some(Runtime::Tokio1), NoTls).unwrap();

                RrDatabase { connection_pool }
            } else {
                unimplemented!()
            }
        } else {
            dotenvy::dotenv().ok();
            let pg_config = PgConfig::from_env().unwrap();
            debug!(target: "server_log", "pg_config settings: {:?}", pg_config);

            let connection_pool = if let Some(ca_cert) = pg_config.db_ca_cert {
                let cert_file = File::open(ca_cert).unwrap();
                let mut buf = BufReader::new(cert_file);
                let mut root_store = rustls::RootCertStore::empty();
                let certs: Vec<_> = rustls_pemfile::certs(&mut buf).collect();
                let my_certs: Vec<_> =
                    certs.iter().map(|c| Certificate(c.as_ref().unwrap().to_vec())).collect();
                for cert in my_certs {
                    root_store.add(&cert).unwrap();
                }

                let tls_config = RustlsClientConfig::builder()
                    .with_safe_defaults()
                    .with_root_certificates(root_store)
                    .with_no_client_auth();

                let tls = MakeRustlsConnect::new(tls_config);
                pg_config.pg.create_pool(Some(Runtime::Tokio1), tls).unwrap()
            } else {
                pg_config.pg.create_pool(Some(Runtime::Tokio1), NoTls).unwrap()
            };

            RrDatabase { connection_pool }
        }
    }

    pub async fn get_connection(&self) -> Result<Client, String> {
        self.connection_pool.get().await.map_err(|err| err.to_string())
    }

    pub async fn get_miner_rewards(
        &self,
        miner_pubkey: String,
    ) -> Result<models::Reward, DatabaseError> {
        let sql = "SELECT r.balance, r.miner_id FROM miners m JOIN rewards r ON m.id = r.miner_id WHERE m.pubkey = $1";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn
                .query_one(&stmt, &[&miner_pubkey])
                .await
                .map(|row| Reward { balance: row.get(0), miner_id: row.get(1) })
                .map_err(From::from);

            res
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    pub async fn get_last_challenge_contributions(
        &self,
    ) -> Result<Vec<ContributionWithPubkey>, DatabaseError> {
        let sql = r#"
SELECT
        s.id                   AS id,
        s.miner_id             AS miner_id,
        s.challenge_id         AS challenge_id,
        s.nonce                AS nonce,
        s.difficulty           AS difficulty,
        s.created              AS created,
        m.pubkey               AS pubkey
    FROM
        contributions s
            INNER JOIN miners m ON s.miner_id = m.id
            INNER JOIN challenges c ON s.challenge_id = c.id
    WHERE c.id = (
        SELECT id FROM challenges 
            WHERE rewards_earned IS NOT NULL
            ORDER BY id DESC LIMIT 1
    )
    ORDER BY id ASC
"#;

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let mut contributions = vec![];
            let c_iter = db_conn.query(&stmt, &[]).await.unwrap().into_iter().map(|row| {
                ContributionWithPubkey {
                    id: row.get("id"),
                    miner_id: row.get("miner_id"),
                    challenge_id: row.get("challenge_id"),
                    nonce: row.get::<&str, i64>("nonce") as u64,
                    difficulty: row.get("difficulty"),
                    created: row.get("created"),
                    pubkey: row.get("pubkey"),
                }
            });

            for c in c_iter {
                contributions.push(c);
            }
            return Ok(contributions);
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    pub async fn get_miner_contributions(
        &self,
        pubkey: String,
    ) -> Result<Vec<Contribution>, DatabaseError> {
        let sql = "SELECT s.* FROM contributions s JOIN miners m ON s.miner_id = m.id WHERE m.pubkey = $1 ORDER BY s.created DESC LIMIT 100";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let mut contributions = vec![];
            let c_iter = db_conn.query(&stmt, &[&pubkey]).await.unwrap().into_iter().map(|row| {
                Contribution {
                    id: row.get("id"),
                    miner_id: row.get("miner_id"),
                    challenge_id: row.get("challenge_id"),
                    nonce: row.get::<&str, i64>("nonce") as u64,
                    difficulty: row.get("difficulty"),
                    created: row.get("created"),
                }
            });

            for c in c_iter {
                contributions.push(c);
            }
            return Ok(contributions);
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    pub async fn get_challenges(&self) -> Result<Vec<ChallengeWithDifficulty>, DatabaseError> {
        let sql = r#"
SELECT
        c.id                   AS id,
        c.rewards_earned       AS rewards_earned,
        c.updated              AS updated,
        s.difficulty           AS difficulty,
    FROM
        challenges c
            INNER JOIN contributions s ON c.contribution_id = s.id
    WHERE c.contribution_id IS NOT NULL
    ORDER BY c.id DESC LIMIT 1440
"#;

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let mut challenges = vec![];
            let c_iter = db_conn.query(&stmt, &[]).await.unwrap().into_iter().map(|row| {
                ChallengeWithDifficulty {
                    id: row.get("id"),
                    rewards_earned: row.get("rewards_earned"),
                    difficulty: row.get("difficulty"),
                    updated: row.get("updated"),
                }
            });

            for c in c_iter {
                challenges.push(c);
            }
            return Ok(challenges);
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    pub async fn get_pool_by_authority_pubkey(
        &self,
        authority_pubkey: String,
    ) -> Result<models::Pool, DatabaseError> {
        let sql = "SELECT id, proof_pubkey, authority_pubkey, total_rewards, claimed_rewards, pool_pubkey FROM pools WHERE pools.authority_pubkey = $1";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn
                .query_one(&stmt, &[&authority_pubkey])
                .await
                .map(|row| models::Pool {
                    id: row.get(0),
                    proof_pubkey: row.get(1),
                    authority_pubkey: row.get(2),
                    total_rewards: row.get(3),
                    claimed_rewards: row.get(4),
                    pool_pubkey: row.get(5),
                })
                .map_err(From::from);

            res
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    pub async fn get_latest_mine_transaction(&self) -> Result<models::Transaction, DatabaseError> {
        let sql = "SELECT * FROM transactions WHERE transaction_type = $1 ORDER BY id DESC LIMIT 1";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn
                .query_one(&stmt, &[&"mine"])
                .await
                .map(|row| models::Transaction {
                    id: row.get("id"),
                    transaction_type: row.get("transaction_type"),
                    signature: row.get("signature"),
                    priority_fee: row.get("priority_fee"),
                    pool_id: row.get("pool_id"),
                    created: row.get("created"),
                })
                .map_err(From::from);

            res
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    pub async fn get_last_claim_by_pubkey(
        &self,
        pubkey: String,
    ) -> Result<LastClaim, DatabaseError> {
        let sql = "SELECT c.created FROM claims c JOIN miners m ON c.miner_id = m.id WHERE m.pubkey = $1 ORDER BY c.id DESC LIMIT 1";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn
                .query_one(&stmt, &[&pubkey])
                .await
                .map(|row| LastClaim { created: row.get(0) })
                .map_err(From::from);

            res
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }
}
