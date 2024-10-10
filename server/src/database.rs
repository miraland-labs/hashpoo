use {
    crate::models::{self, *},
    // crate::utils,
    std::{fmt, io, str::FromStr},
    tracing::{error, info, warn},
};
#[cfg(feature = "powered-by-dbms-sqlite")]
use {
    crate::utils,
    deadpool_sqlite::{Config, Connection, Pool, Runtime},
    rusqlite::params,
};
#[cfg(feature = "powered-by-dbms-postgres")]
use {
    deadpool_postgres::{Client, Config, ManagerConfig, Pool, RecyclingMethod, Runtime},
    pg_connection_string::{from_multi_str, ConnectionString},
    rustls::{Certificate, ClientConfig as RustlsClientConfig},
    std::{fs::File, io::BufReader},
    tokio_postgres::{error::SqlState, Error as PgError, NoTls},
    tokio_postgres_rustls::MakeRustlsConnect,
    tracing::debug,
};

pub struct PoweredByParams<'p> {
    // File path for the ore private pool database file relative to current directory
    // pub db_file_path: &'p str,
    pub database_uri: &'p str,
    pub database_rr_uri: &'p str,
    pub initialized: bool,
    pub corrupted: bool,
    // pub connection: Option<Connection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PoweredByDbms {
    Mysql,
    Postgres,
    Sqlite,
    Unavailable,
}

impl fmt::Display for PoweredByDbms {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            PoweredByDbms::Mysql => write!(f, "mysql"),
            PoweredByDbms::Postgres => write!(f, "postgres"),
            PoweredByDbms::Sqlite => write!(f, "sqlite3"),
            PoweredByDbms::Unavailable => write!(f, "unavailable"),
        }
    }
}

impl FromStr for PoweredByDbms {
    type Err = io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "mysql" => Ok(PoweredByDbms::Mysql),
            "postgres" => Ok(PoweredByDbms::Postgres),
            "sqlite3" => Ok(PoweredByDbms::Sqlite),
            _ => Ok(PoweredByDbms::Unavailable),
            // _ => Err(io::Error::new(io::ErrorKind::InvalidData, "Unsupported dbms type")),
        }
    }
}

#[derive(Debug)]
pub enum DatabaseError {
    FailedToGetConnectionFromPool,
    FailedToUpdateRow,
    FailedToInsertRow,
    #[cfg(feature = "powered-by-dbms-sqlite")]
    InteractionFailed,
    QueryFailed,
}

#[cfg(feature = "powered-by-dbms-postgres")]
/// Utility function for simply mapping any underline db error to DatabaseError
impl From<PgError> for DatabaseError {
    fn from(e: PgError) -> Self {
        match e.code() {
            // MI: for query_one(), tokio_postgres returns 0 or more than 1 rows result as Error::RowCount with cause None
            None => {
                warn!(target: "server_log", "query_one() returned no rows.");
                DatabaseError::QueryFailed
            },
            Some(&SqlState::NO_DATA_FOUND) => {
                warn!(target: "server_log", "Query returned no rows.");
                DatabaseError::QueryFailed
            },
            Some(err_code) => {
                error!(target: "server_log", "Query error: {:?}", err_code);
                DatabaseError::QueryFailed
            },
        }
    }
}

#[cfg(feature = "powered-by-dbms-postgres")]
#[derive(Debug, serde::Deserialize, serde::Serialize)]
pub(crate) struct PgConfig {
    pub(crate) db_ca_cert: Option<String>,
    pub(crate) listen: Option<String>,
    pub(crate) pg: deadpool_postgres::Config,
}

#[cfg(feature = "powered-by-dbms-postgres")]
impl PgConfig {
    pub fn from_env() -> Result<Self, config::ConfigError> {
        config::Config::builder()
            .add_source(config::Environment::default().separator("__"))
            .build()
            .unwrap()
            .try_deserialize()
    }
}

#[cfg(feature = "powered-by-dbms-sqlite")]
pub async fn validate_dbms(dbms_settings: &mut PoweredByParams<'_>, _database: Database) -> bool {
    // First, let's check if db file exist or not
    if !utils::exists_file(dbms_settings.database_uri) {
        warn!(target: "server_log",
            "No existing database! New database will be created in the path: {}",
            dbms_settings.database_uri
        );
    } else {
        info!(target: "server_log",
            "The ore pool sqlite db is already in place: {}. Opening...",
            dbms_settings.database_uri
        );
        dbms_settings.initialized = true;
    }
    // Second, we try to open a database connection.
    let conn = match rusqlite::Connection::open(dbms_settings.database_uri) {
        Ok(conn) => conn,
        Err(e) => {
            error!(target: "server_log", "Error connecting to database: {}.", e);
            return false;
        },
    };

    // initialization check
    if !dbms_settings.initialized {
        info!(target: "server_log", "Initializing database...");
        // execute db init sql scripts
        // let command = fs::read_to_string("migrations/sqlite/init.sql").unwrap();
        let command = include_str!("../migrations/sqlite/init.sql");
        if let Err(e) = conn.execute_batch(&command) {
            error!(target: "server_log", "Error occurred during db initialization: {}", e);
            return false;
        }
        dbms_settings.initialized = true;
        info!(target: "server_log", "Initialization completed.");
    }

    // retrive initialization completed flag
    let mut stmt =
        conn.prepare("SELECT id FROM init_completion WHERE init_completed = true").unwrap();
    dbms_settings.corrupted = !stmt.exists([]).unwrap();

    // db file corruption check
    if dbms_settings.corrupted {
        error!(target: "server_log", "ore pool db file corrupted.");
        return false;
    }

    true
}

#[cfg(feature = "powered-by-dbms-postgres")]
pub async fn validate_dbms(dbms_settings: &mut PoweredByParams<'_>, database: Database) -> bool {
    // First, we try to get a database connection.
    let mut conn: Client;
    if let Ok(c) = database.get_connection().await {
        conn = c;
    } else {
        error!(target: "server_log", "Error fetching connection from pool, cannot connect to database.");
        return false;
    }

    info!(target: "server_log", "Connecting to database...");

    // initialization check
    let check_init_sql =
        "SELECT 1 FROM information_schema.tables WHERE table_name = 'init_completion'";
    if let Err(_) = conn.query_one(check_init_sql, &[]).await {
        // warn!(target: "server_log", "ore pool db has not been initialized: {}", e);
        warn!(target: "server_log", "ore pool db has not been initialized.");
        // info!(target: "server_log", "Prepare initializing...");
        dbms_settings.initialized = false;
    } else {
        dbms_settings.initialized = true;
    }

    // match conn.query_opt(check_init_sql, &[]).await {
    //     // more than 1 rows returned
    //     Err(e) => {
    //         error!(target: "server_log", "ore pool db has not been initialized: {}", e);
    //         dbms_settings.initialized = false;
    //     },
    //     // None for no rows
    //     Ok(None) => {
    //         error!(target: "server_log", "Not found init_completion table. ore pool db has not been initialized.");
    //         dbms_settings.initialized = false;
    //     },
    //     // Found exact one
    //     Ok(_) => dbms_settings.initialized = true,
    // }

    if !dbms_settings.initialized {
        info!(target: "server_log", "Initializing database...");
        // execute db init sql scripts
        // let command = fs::read_to_string("migrations/postgres/init.sql").unwrap();
        let init_sql = include_str!("../migrations/postgres/init.sql");
        let tx = conn.transaction().await.unwrap();
        // if let Err(e) = tx.execute(init_sql, &[]).await {
        if let Err(e) = tx.batch_execute(init_sql).await {
            error!(target: "server_log", "Error occurred during db initialization: {}", e);
            return false;
        }
        tx.commit().await.unwrap();

        dbms_settings.initialized = true;
        info!(target: "server_log", "Initialization completed.");
    }

    // retrive initialization completed flag
    let check_comp_flag_sql = "SELECT id FROM init_completion WHERE init_completed = true";
    if let Err(e) = conn.query_one(check_comp_flag_sql, &[]).await {
        error!(target: "server_log", "ore pool db corrupted. {}", e);
        dbms_settings.corrupted = true;
        return false;
    }
    dbms_settings.corrupted = false;

    // match conn.query_opt(check_comp_flag_sql, &[]).await {
    //     // more than 1 rows returned
    //     Err(e) => {
    //         error!(target: "server_log", "ore pool db corrupted: {}", e);
    //         dbms_settings.corrupted = true;
    //         return false;
    //     },
    //     // None for no rows
    //     Ok(None) => {
    //         error!(target: "server_log", "No rows found. ore pool db corrupted.");
    //         dbms_settings.corrupted = true;
    //         return false;
    //     },
    //     // Found exact one
    //     Ok(_) => dbms_settings.corrupted = false,
    // }

    true
}

#[derive(Debug, Clone)]
pub struct Database {
    connection_pool: Pool,
}

impl Database {
    #[cfg(feature = "powered-by-dbms-postgres")]
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

                Database { connection_pool }
            } else {
                unimplemented!()
            }
        } else {
            dotenvy::dotenv().ok();
            let pg_config = PgConfig::from_env().unwrap();
            debug!(target: "server_log", "pg_config settings: {:?}", pg_config);
            // let connection_pool =
            //     pg_config.pg.create_pool(Some(Runtime::Tokio1), tokio_postgres::NoTls).unwrap();

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

                // for cert in rustls_pemfile::certs(&mut buf) {
                //     root_store.add(&cert.unwrap())?;
                // }

                let tls_config = RustlsClientConfig::builder()
                    .with_safe_defaults()
                    .with_root_certificates(root_store)
                    .with_no_client_auth();

                let tls = MakeRustlsConnect::new(tls_config);
                pg_config.pg.create_pool(Some(Runtime::Tokio1), tls).unwrap()
            } else {
                pg_config.pg.create_pool(Some(Runtime::Tokio1), NoTls).unwrap()
            };

            Database { connection_pool }
        }
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub fn new(database_uri: String) -> Self {
        let db_config = Config::new(database_uri);
        let connection_pool = db_config.create_pool(Runtime::Tokio1).unwrap();

        Database { connection_pool }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn get_connection(&self) -> Result<Client, String> {
        // let client: Client = pool.get().await?;
        self.connection_pool.get().await.map_err(|err| err.to_string())
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn _get_connection(&self) -> Result<Connection, String> {
        self.connection_pool.get().await.map_err(|err| err.to_string())
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn get_pool_by_authority_pubkey(
        &self,
        authority_pubkey: String,
    ) -> Result<models::Pool, DatabaseError> {
        let sql = "SELECT id, proof_pubkey, authority_pubkey, total_rewards, claimed_rewards, pool_pubkey FROM pools WHERE pools.authority_pubkey = ?";

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    let row = conn.query_row_and_then(sql, &[&authority_pubkey], |row| {
                        Ok::<models::Pool, rusqlite::Error>(models::Pool {
                            id: row.get(0)?,
                            proof_pubkey: row.get(1)?,
                            authority_pubkey: row.get(2)?,
                            total_rewards: row.get(3)?,
                            claimed_rewards: row.get(4)?,
                            pool_pubkey: row.get(5)?,
                        })
                    });
                    row
                })
                .await;

            match res {
                Ok(Ok(pool)) => Ok(pool),
                Ok(Err(e)) => match e {
                    rusqlite::Error::QueryReturnedNoRows => {
                        warn!(target: "server_log", "Query returned no rows.");
                        return Err(DatabaseError::QueryFailed);
                    },
                    e => {
                        error!(target: "server_log", "Query error: {}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
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

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn add_new_pool(
        &self,
        authority_pubkey: String,
        proof_pubkey: String,
        pool_pubkey: String,
    ) -> Result<(), DatabaseError> {
        let sql =
            "INSERT INTO pools (authority_pubkey, proof_pubkey, pool_pubkey) VALUES (?, ?, ?)";

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    conn.execute(sql, &[&authority_pubkey, &proof_pubkey, &pool_pubkey])
                })
                .await;

            match res {
                Ok(interaction) => match interaction {
                    Ok(query) => {
                        if query != 1 {
                            return Err(DatabaseError::FailedToInsertRow);
                        }
                        return Ok(());
                    },
                    Err(e) => {
                        error!(target: "server_log", "{:?}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        };
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn add_new_pool(
        &self,
        authority_pubkey: String,
        proof_pubkey: String,
        pool_pubkey: String,
    ) -> Result<(), DatabaseError> {
        let sql =
            "INSERT INTO pools (authority_pubkey, proof_pubkey, pool_pubkey) VALUES ($1, $2, $3)";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res =
                db_conn.execute(&stmt, &[&authority_pubkey, &proof_pubkey, &pool_pubkey]).await;

            match res {
                Ok(num_rows) => {
                    if num_rows != 1 {
                        return Err(DatabaseError::FailedToInsertRow);
                    }
                    return Ok(());
                },
                Err(e) => {
                    error!(target: "server_log", "{}", e);
                    return Err(DatabaseError::QueryFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn update_pool_rewards(
        &self,
        pool_authority_pubkey: String,
        earned_rewards: i64,
    ) -> Result<(), DatabaseError> {
        let sql = "UPDATE pools SET total_rewards = total_rewards + ? WHERE authority_pubkey = ?";

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    conn.execute(
                        sql,
                        // &[&earned_rewards, &pool_authority_pubkey]
                        params![&earned_rewards, &pool_authority_pubkey],
                    )
                })
                .await;

            match res {
                Ok(interaction) => match interaction {
                    Ok(query) => {
                        if query != 1 {
                            return Err(DatabaseError::FailedToUpdateRow);
                        }
                        info!(target: "server_log", "Successfully updated pool rewards");
                        return Ok(());
                    },
                    Err(e) => {
                        error!(target: "server_log", "{:?}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn update_pool_rewards(
        &self,
        pool_authority_pubkey: String,
        earned_rewards: i64,
    ) -> Result<(), DatabaseError> {
        let sql = "UPDATE pools SET total_rewards = total_rewards + $1 WHERE authority_pubkey = $2";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn.execute(&stmt, &[&earned_rewards, &pool_authority_pubkey]).await;

            match res {
                Ok(num_rows) => {
                    if num_rows != 1 {
                        return Err(DatabaseError::FailedToUpdateRow);
                    }
                    info!(target: "server_log", "Successfully updated pool rewards");
                    return Ok(());
                },
                Err(e) => {
                    error!(target: "server_log", "{}", e);
                    return Err(DatabaseError::QueryFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn update_pool_claimed(
        &self,
        pool_authority_pubkey: String,
        claimed_rewards: i64,
    ) -> Result<(), DatabaseError> {
        let sql =
            "UPDATE pools SET claimed_rewards = claimed_rewards + $1 WHERE authority_pubkey = $2";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn.execute(&stmt, &[&claimed_rewards, &pool_authority_pubkey]).await;

            match res {
                Ok(num_rows) => {
                    if num_rows != 1 {
                        return Err(DatabaseError::FailedToUpdateRow);
                    }
                    info!(target: "server_log", "Successfully updated pool rewards");
                    return Ok(());
                },
                Err(e) => {
                    error!(target: "server_log", "{}", e);
                    return Err(DatabaseError::QueryFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn add_new_miner(
        &self,
        miner_pubkey: String,
        is_enabled: bool,
        status: String,
    ) -> Result<(), DatabaseError> {
        let sql = "INSERT INTO miners (pubkey, enabled, status) VALUES (?, ?, ?)";

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    conn.execute(
                        sql,
                        // &[&miner_pubkey, &is_enabled, &status]
                        params![&miner_pubkey, &is_enabled, &status],
                    )
                })
                .await;

            match res {
                Ok(interaction) => match interaction {
                    Ok(query) => {
                        if query != 1 {
                            return Err(DatabaseError::FailedToInsertRow);
                        }
                        return Ok(());
                    },
                    Err(e) => {
                        error!(target: "server_log", "{:?}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn add_new_miner(
        &self,
        miner_pubkey: String,
        is_enabled: bool,
        status: String,
    ) -> Result<(), DatabaseError> {
        let sql = "INSERT INTO miners (pubkey, enabled, status) VALUES ($1, $2, $3)";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn.execute(&stmt, &[&miner_pubkey, &is_enabled, &status]).await;

            match res {
                Ok(num_rows) => {
                    if num_rows != 1 {
                        return Err(DatabaseError::FailedToInsertRow);
                    }
                    return Ok(());
                },
                Err(e) => {
                    error!(target: "server_log", "{}", e);
                    return Err(DatabaseError::QueryFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn get_miner_by_pubkey_str(
        &self,
        miner_pubkey: String,
    ) -> Result<Miner, DatabaseError> {
        let sql = "SELECT id, pubkey, enabled, status FROM miners WHERE miners.pubkey = ?";

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    let row = conn.query_row_and_then(sql, &[&miner_pubkey], |row| {
                        Ok::<Miner, rusqlite::Error>(Miner {
                            id: row.get(0)?,
                            pubkey: row.get(1)?,
                            enabled: row.get(2)?,
                            status: row.get(3)?,
                        })
                    });
                    row
                })
                .await;

            match res {
                Ok(Ok(miner)) => Ok(miner),
                Ok(Err(e)) => match e {
                    rusqlite::Error::QueryReturnedNoRows => {
                        warn!(target: "server_log", "Query returned no rows.");
                        return Err(DatabaseError::QueryFailed);
                    },
                    e => {
                        error!(target: "server_log", "Query error: {}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn get_miner_by_pubkey_str(
        &self,
        miner_pubkey: String,
    ) -> Result<Miner, DatabaseError> {
        let sql = "SELECT id, pubkey, enabled, status FROM miners WHERE miners.pubkey = $1";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn
                .query_one(&stmt, &[&miner_pubkey])
                .await
                .map(|row| Miner {
                    id: row.get(0),
                    pubkey: row.get(1),
                    enabled: row.get(2),
                    status: row.get(3),
                })
                .map_err(From::from);

            res
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn get_challenge_by_challenge(
        &self,
        challenge: Vec<u8>,
    ) -> Result<Challenge, DatabaseError> {
        let sql = "SELECT id, pool_id, contribution_id, challenge, rewards_earned FROM challenges WHERE challenges.challenge = ?";

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    let row = conn.query_row_and_then(sql, &[&challenge], |row| {
                        Ok::<Challenge, rusqlite::Error>(Challenge {
                            id: row.get(0)?,
                            pool_id: row.get(1)?,
                            contribution_id: row.get(2)?,
                            challenge: row.get(3)?,
                            rewards_earned: row.get(4)?,
                        })
                    });
                    row
                })
                .await;

            match res {
                Ok(Ok(challenge)) => Ok(challenge),
                Ok(Err(e)) => match e {
                    rusqlite::Error::QueryReturnedNoRows => {
                        warn!(target: "server_log", "Query returned no rows.");
                        return Err(DatabaseError::QueryFailed);
                    },
                    e => {
                        error!(target: "server_log", "Query error: {}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn get_challenge_by_challenge(
        &self,
        challenge: Vec<u8>,
    ) -> Result<Challenge, DatabaseError> {
        let sql = "SELECT id, pool_id, contribution_id, challenge, rewards_earned FROM challenges WHERE challenges.challenge = $1";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn
                .query_one(&stmt, &[&challenge])
                .await
                .map(|row| Challenge {
                    id: row.get(0),
                    pool_id: row.get(1),
                    contribution_id: row.get(2),
                    challenge: row.get(3),
                    rewards_earned: row.get(4),
                })
                .map_err(From::from);

            res
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn get_contribution_id_with_nonce(&self, nonce: u64) -> Result<i64, DatabaseError> {
        let sql = "SELECT id FROM contributions WHERE contributions.nonce = ? ORDER BY id DESC";

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    let row = conn.query_row_and_then(sql, &[&(nonce as i64)], |row| {
                        Ok::<i64, rusqlite::Error>(row.get(0)?)
                    });
                    row
                })
                .await;

            match res {
                Ok(Ok(contribution_id)) => Ok(contribution_id),
                Ok(Err(e)) => match e {
                    rusqlite::Error::QueryReturnedNoRows => {
                        error!(target: "server_log", "Query returned no rows.");
                        return Err(DatabaseError::QueryFailed);
                    },
                    e => {
                        error!(target: "server_log", "Query error: {}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn get_contribution_id_with_nonce(&self, nonce: u64) -> Result<i64, DatabaseError> {
        let sql = "SELECT id FROM contributions WHERE contributions.nonce = $1 ORDER BY id DESC";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn
                .query_one(&stmt, &[&(nonce as i64)])
                .await
                .map(|row| row.get(0))
                .map_err(From::from);

            res
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn update_challenge_rewards(
        &self,
        challenge: Vec<u8>,
        contribution_id: i64,
        rewards: i64,
    ) -> Result<(), DatabaseError> {
        let sql =
            "UPDATE challenges SET rewards_earned = ?, contribution_id = ? WHERE challenge = ?";

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    conn.execute(
                        sql,
                        // &[&pool_authority_pubkey, &earned_rewards]
                        params![&rewards, &contribution_id, &challenge],
                    )
                })
                .await;

            match res {
                Ok(interaction) => match interaction {
                    Ok(query) => {
                        if query != 1 {
                            return Err(DatabaseError::FailedToUpdateRow);
                        }
                        info!(target: "server_log", "Successfully updated challenge rewards");
                        return Ok(());
                    },
                    Err(e) => {
                        error!(target: "server_log", "{:?}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn update_challenge_rewards(
        &self,
        challenge: Vec<u8>,
        contribution_id: i64,
        rewards: i64,
    ) -> Result<(), DatabaseError> {
        let sql =
            "UPDATE challenges SET rewards_earned = $1, contribution_id = $2 WHERE challenge = $3";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn.execute(&stmt, &[&rewards, &contribution_id, &challenge]).await;

            match res {
                Ok(num_rows) => {
                    if num_rows != 1 {
                        return Err(DatabaseError::FailedToUpdateRow);
                    }
                    info!(target: "server_log", "Successfully updated challenge rewards");
                    return Ok(());
                },
                Err(e) => {
                    error!(target: "server_log", "{}", e);
                    return Err(DatabaseError::QueryFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn add_new_challenge(&self, challenge: InsertChallenge) -> Result<(), DatabaseError> {
        let sql = r#"INSERT INTO challenges (pool_id, challenge, rewards_earned) VALUES (?, ?, ?)"#;

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    conn.execute(
                        sql,
                        // &[&challenge.pool_id, &challenge.challenge, &challenge.rewards_earned],
                        params![challenge.pool_id, challenge.challenge, challenge.rewards_earned],
                    )
                })
                .await;

            match res {
                Ok(interaction) => match interaction {
                    Ok(query) => {
                        if query != 1 {
                            return Err(DatabaseError::FailedToInsertRow);
                        }
                        return Ok(());
                    },
                    Err(e) => {
                        error!(target: "server_log", "{:?}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn add_new_challenge(&self, challenge: InsertChallenge) -> Result<(), DatabaseError> {
        let sql =
            r#"INSERT INTO challenges (pool_id, challenge, rewards_earned) VALUES ($1, $2, $3)"#;

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn
                .execute(
                    &stmt,
                    &[&challenge.pool_id, &challenge.challenge, &challenge.rewards_earned],
                )
                .await;

            match res {
                Ok(num_rows) => {
                    if num_rows != 1 {
                        return Err(DatabaseError::FailedToInsertRow);
                    }
                    return Ok(());
                },
                Err(e) => {
                    error!(target: "server_log", "{}", e);
                    return Err(DatabaseError::QueryFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn add_new_claim(&self, claim: InsertClaim) -> Result<(), DatabaseError> {
        let sql = r#"INSERT INTO claims (miner_id, pool_id, transaction_id, amount) VALUES ($1, $2, $3, $4)"#;

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn
                .execute(
                    &stmt,
                    &[&claim.miner_id, &claim.pool_id, &claim.transaction_id, &claim.amount],
                )
                .await;

            match res {
                Ok(num_rows) => {
                    if num_rows != 1 {
                        return Err(DatabaseError::FailedToInsertRow);
                    }
                    return Ok(());
                },
                Err(e) => {
                    error!(target: "server_log", "{}", e);
                    return Err(DatabaseError::QueryFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn get_last_claim(&self, miner_id: i64) -> Result<LastClaim, DatabaseError> {
        let sql = r#"SELECT created FROM claims WHERE miner_id = $1 ORDER BY id DESC LIMIT 1"#;

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn
                .query_one(&stmt, &[&miner_id])
                .await
                .map(|row| LastClaim { created: row.get(0) })
                .map_err(From::from);

            res
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn add_new_transaction(&self, txn: InsertTransaction) -> Result<(), DatabaseError> {
        let sql = r#"INSERT INTO transactions (transaction_type, signature, priority_fee, pool_id) VALUES (?, ?, ?, ?)"#;

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    conn.execute(
                        sql,
                        // &[&txn.transaction_type, &txn.signature, &txn.priority_fee,
                        // &txn.pool_id],
                        params![txn.transaction_type, txn.signature, txn.priority_fee, txn.pool_id],
                    )
                })
                .await;

            match res {
                Ok(interaction) => match interaction {
                    Ok(query) => {
                        if query != 1 {
                            return Err(DatabaseError::FailedToInsertRow);
                        }
                        return Ok(());
                    },
                    Err(e) => {
                        error!(target: "server_log", "{:?}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn add_new_transaction(&self, txn: InsertTransaction) -> Result<(), DatabaseError> {
        let sql = r#"INSERT INTO transactions (transaction_type, signature, priority_fee, pool_id) VALUES ($1, $2, $3, $4)"#;

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn
                .execute(
                    &stmt,
                    &[&txn.transaction_type, &txn.signature, &txn.priority_fee, &txn.pool_id],
                )
                .await;

            match res {
                Ok(num_rows) => {
                    if num_rows != 1 {
                        return Err(DatabaseError::FailedToInsertRow);
                    }
                    return Ok(());
                },
                Err(e) => {
                    error!(target: "server_log", "{}", e);
                    return Err(DatabaseError::QueryFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn get_transaction_id_by_sig(
        &self,
        sig: String,
    ) -> Result<TransactionId, DatabaseError> {
        let sql = "SELECT id FROM transaction WHERE signature = $1";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn
                .query_one(&stmt, &[&sig])
                .await
                .map(|row| TransactionId { id: row.get(0) })
                .map_err(From::from);

            res
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn add_new_earnings_batch(
        &self,
        earnings: Vec<InsertEarning>,
    ) -> Result<(), DatabaseError> {
        let sql =
            r#"INSERT INTO earnings (miner_id, pool_id, challenge_id, amount) VALUES (?, ?, ?, ?)"#;

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    // conn.execute(
                    //     sql,
                    //     &[&earnings.miner_id, &earings.pool_id, &earnings.challenge_id,
                    // &earnings.amount], )

                    let mut rows_affected = 0;
                    // or conn.unchecked_transaction() if you don't have &mut Connection
                    let tx = conn.transaction().unwrap();
                    let mut stmt = tx.prepare(sql).unwrap();
                    for earning in &earnings {
                        rows_affected += stmt
                            .execute((
                                &earning.miner_id,
                                &earning.pool_id,
                                &earning.challenge_id,
                                &earning.amount,
                            ))
                            .unwrap();
                    }
                    drop(stmt);
                    tx.commit().unwrap();

                    Ok::<usize, rusqlite::Error>(rows_affected)
                })
                .await;

            match res {
                // Ok(()) => Ok(()),
                Ok(interaction) => match interaction {
                    Ok(rows) => {
                        info!(target: "server_log", "Earnings inserted: {}", rows);
                        if rows == 0 {
                            return Err(DatabaseError::FailedToInsertRow);
                        }
                        return Ok(());
                    },
                    Err(e) => {
                        error!(target: "server_log", "{:?}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn add_new_earnings_batch(
        &self,
        earnings: Vec<InsertEarning>,
    ) -> Result<(), DatabaseError> {
        let sql = r#"INSERT INTO earnings (miner_id, pool_id, challenge_id, amount) VALUES ($1, $2, $3, $4)"#;

        if let Ok(mut db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();

            let mut rows_affected = 0;
            let tx = db_conn.transaction().await.unwrap();
            for earning in &earnings {
                rows_affected += tx
                    .execute(
                        &stmt,
                        &[
                            &earning.miner_id,
                            &earning.pool_id,
                            &earning.challenge_id,
                            &earning.amount,
                        ],
                    )
                    .await
                    .unwrap();
            }
            tx.commit().await.unwrap();

            info!(target: "server_log", "Earnings inserted: {}", rows_affected);
            if rows_affected == 0 {
                return Err(DatabaseError::FailedToInsertRow);
            }
            return Ok(());
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn add_new_contributions_batch(
        &self,
        contributions: Vec<InsertContribution>,
    ) -> Result<(), DatabaseError> {
        let sql = r#"INSERT INTO contributions (miner_id, challenge_id, nonce, digest, difficulty) VALUES (?, ?, ?, ?, ?)"#;

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    let mut rows_affected = 0;
                    // or conn.unchecked_transaction() if you don't have &mut Connection
                    let tx = conn.transaction().unwrap();
                    let mut stmt = tx.prepare(sql).unwrap();
                    for contribution in &contributions {
                        rows_affected += stmt
                            .execute((
                                &contribution.miner_id,
                                &contribution.challenge_id,
                                &(contribution.nonce as i64),
                                &contribution.digest,
                                &contribution.difficulty,
                            ))
                            .unwrap();
                    }
                    drop(stmt);
                    tx.commit().unwrap();

                    Ok::<usize, rusqlite::Error>(rows_affected)
                })
                .await;

            match res {
                // Ok(()) => Ok(()),
                Ok(interaction) => match interaction {
                    Ok(rows) => {
                        info!(target: "server_log", "Contributions inserted: {}", rows);
                        if rows == 0 {
                            return Err(DatabaseError::FailedToInsertRow);
                        }
                        return Ok(());
                    },
                    Err(e) => {
                        error!(target: "server_log", "{:?}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn add_new_contributions_batch(
        &self,
        contributions: Vec<InsertContribution>,
    ) -> Result<(), DatabaseError> {
        let sql = r#"INSERT INTO contributions (miner_id, challenge_id, nonce, digest, difficulty) VALUES ($1, $2, $3, $4, $5)"#;

        if let Ok(mut db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();

            let mut rows_affected = 0;
            let tx = db_conn.transaction().await.unwrap();
            for contribution in &contributions {
                rows_affected += tx
                    .execute(
                        &stmt,
                        &[
                            &contribution.miner_id,
                            &contribution.challenge_id,
                            &(contribution.nonce as i64),
                            &contribution.digest,
                            &contribution.difficulty,
                        ],
                    )
                    .await
                    .unwrap();
            }
            tx.commit().await.unwrap();

            info!(target: "server_log", "Contributions inserted: {}", rows_affected);
            if rows_affected == 0 {
                return Err(DatabaseError::FailedToInsertRow);
            }
            return Ok(());
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn _get_miner_rewards(&self, miner_pubkey: String) -> Result<Reward, DatabaseError> {
        let sql = r#"SELECT r.balance, r.miner_id FROM miners m JOIN rewards r ON m.id = r.miner_id WHERE m.pubkey = ?"#;

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    let row = conn.query_row_and_then(sql, &[&miner_pubkey], |row| {
                        Ok::<Reward, rusqlite::Error>(Reward {
                            balance: row.get(0)?,
                            miner_id: row.get(1)?,
                        })
                    });
                    row
                })
                .await;

            match res {
                Ok(Ok(reward)) => Ok(reward),
                Ok(Err(e)) => match e {
                    rusqlite::Error::QueryReturnedNoRows => {
                        error!(target: "server_log", "Query returned no rows.");
                        return Err(DatabaseError::QueryFailed);
                    },
                    e => {
                        error!(target: "server_log", "Query error: {}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn get_miner_rewards(&self, miner_pubkey: String) -> Result<Reward, DatabaseError> {
        let sql = r#"SELECT r.balance, r.miner_id FROM miners m JOIN rewards r ON m.id = r.miner_id WHERE m.pubkey = $1"#;

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

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn add_new_reward(&self, reward: InsertReward) -> Result<(), DatabaseError> {
        let sql = r#"INSERT INTO rewards (miner_id, pool_id) VALUES (?, ?)"#;

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    conn.execute(
                        sql,
                        // &[&reward.miner_id, &reward.pool_id],
                        params![reward.miner_id, reward.pool_id,],
                    )
                })
                .await;

            match res {
                Ok(interaction) => match interaction {
                    Ok(query) => {
                        if query != 1 {
                            return Err(DatabaseError::FailedToInsertRow);
                        }
                        return Ok(());
                    },
                    Err(e) => {
                        error!(target: "server_log", "{:?}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn add_new_reward(&self, reward: InsertReward) -> Result<(), DatabaseError> {
        let sql = r#"INSERT INTO rewards (miner_id, pool_id) VALUES ($1, $2)"#;

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn.execute(&stmt, &[&reward.miner_id, &reward.pool_id]).await;

            match res {
                Ok(num_rows) => {
                    if num_rows != 1 {
                        return Err(DatabaseError::FailedToInsertRow);
                    }
                    return Ok(());
                },
                Err(e) => {
                    error!(target: "server_log", "{}", e);
                    return Err(DatabaseError::QueryFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn update_rewards(&self, rewards: Vec<UpdateReward>) -> Result<(), DatabaseError> {
        let sql = r#"UPDATE rewards SET balance = balance + ? WHERE miner_id = ?"#;

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    let mut rows_affected = 0;
                    // or conn.unchecked_transaction() if you don't have &mut Connection
                    let tx = conn.transaction().unwrap();
                    let mut stmt = tx.prepare(sql).unwrap();

                    for reward in rewards {
                        rows_affected += stmt.execute((&reward.balance, &reward.miner_id)).unwrap();
                    }
                    drop(stmt);
                    tx.commit().unwrap();

                    Ok::<usize, rusqlite::Error>(rows_affected)
                })
                .await;

            match res {
                // Ok(()) => Ok(()),
                Ok(interaction) => match interaction {
                    Ok(rows) => {
                        info!(target: "server_log", "Rewards updated: {}", rows);
                        if rows == 0 {
                            return Err(DatabaseError::FailedToUpdateRow);
                        }
                        return Ok(());
                    },
                    Err(e) => {
                        error!(target: "server_log", "{:?}", e);
                        return Err(DatabaseError::QueryFailed);
                    },
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn update_rewards(&self, rewards: Vec<UpdateReward>) -> Result<(), DatabaseError> {
        let sql = r#"UPDATE rewards SET balance = balance + $1 WHERE miner_id = $2"#;

        if let Ok(mut db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();

            let mut rows_affected = 0;
            let tx = db_conn.transaction().await.unwrap();
            for reward in rewards {
                rows_affected +=
                    tx.execute(&stmt, &[&reward.balance, &reward.miner_id]).await.unwrap();
            }

            tx.commit().await.unwrap();

            info!(target: "server_log", "Rewards updated: {}", rows_affected);
            if rows_affected == 0 {
                return Err(DatabaseError::FailedToUpdateRow);
            }
            return Ok(());
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn decrease_miner_reward(
        &self,
        miner_id: i64,
        rewards_to_decrease: i64,
    ) -> Result<(), DatabaseError> {
        let sql = "UPDATE rewards SET balance = balance - $1 WHERE miner_id = $2";

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let res = db_conn.execute(&stmt, &[&rewards_to_decrease, &miner_id]).await;

            match res {
                Ok(num_rows) => {
                    if num_rows != 1 {
                        return Err(DatabaseError::FailedToUpdateRow);
                    }
                    info!(target: "server_log", "Successfully decrease miner rewards");
                    return Ok(());
                },
                Err(e) => {
                    error!(target: "server_log", "{}", e);
                    return Err(DatabaseError::QueryFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-sqlite")]
    pub async fn get_summaries_for_last_24_hours(
        &self,
        pool_id: i32,
    ) -> Result<Vec<Summary>, DatabaseError> {
        let sql = r#"
SELECT
        m.pubkey                                      as miner_pubkey,
        COUNT(c.id)                                   as num_of_contributions,
        MIN(c.difficulty)                             as min_diff,
        ROUND(CAST(AVG(c.difficulty) AS REAL), 2)     as avg_diff,
        MAX(c.difficulty)                             as max_diff,
        SUM(e.amount)                                 as earning_sub_total,
        ROUND(CAST(SUM(e.amount) AS REAL) / CAST(SUM(SUM(e.amount)) OVER () AS REAL) * 100, 2) AS percent
    FROM
        contributions c
            INNER JOIN miners m ON c.miner_id = m.id
            INNER JOIN earnings e ON c.challenge_id = e.challenge_id AND c.miner_id = e.miner_id AND e.pool_id = ?
            INNER JOIN pools p ON e.pool_id = p.id
    WHERE
        c.created >= datetime('now', '-24 hour') and
        c.created < 'now' and
        m.enabled = true
    GROUP BY m.pubkey
    ORDER BY percent DESC
        "#;

        if let Ok(db_conn) = self.connection_pool.get().await {
            let res = db_conn
                .interact(move |conn| {
                    // let row = conn.query_row_and_then(sql, &[&pool_id], |row| {
                    //     Ok::<Summary, rusqlite::Error>(Summary {
                    //         miner_pubkey: row.get(0)?,
                    //         num_of_contributions: row.get(1)?,
                    //         min_diff: row.get(2)?,
                    //         avg_diff: row.get(3)?,
                    //         max_diff: row.get(4)?,
                    //         earning_sub_total: row.get(5)?,
                    //         percent: row.get(6)?,
                    //     })
                    // });
                    // row

                    let mut summaries = vec![];
                    let mut stmt = conn.prepare(sql).unwrap();
                    let summary_iter = stmt
                        .query_map(&[&pool_id], |row| {
                            Ok::<Summary, rusqlite::Error>(Summary {
                                miner_pubkey: row.get(0)?,
                                num_of_contributions: row.get(1)?,
                                min_diff: row.get(2)?,
                                avg_diff: row.get(3)?,
                                max_diff: row.get(4)?,
                                earning_sub_total: row.get(5)?,
                                percent: row.get(6)?,
                            })
                        })
                        .unwrap();

                    for summary in summary_iter {
                        summaries.push(summary);
                    }
                    summaries
                })
                .await;

            match res {
                Ok(items) => match &items[..] {
                    [Ok(_), ..] => Ok(items.into_iter().map(|s| s.unwrap()).collect()),
                    [Err(e), ..] => match e {
                        rusqlite::Error::QueryReturnedNoRows => {
                            warn!(target: "server_log", "Query returned no rows.");
                            return Err(DatabaseError::QueryFailed);
                        },
                        e => {
                            error!(target: "server_log", "Query error: {}", e);
                            return Err(DatabaseError::QueryFailed);
                        },
                    },
                    [] => todo!(),
                },
                Err(e) => {
                    error!(target: "server_log", "{:?}", e);
                    return Err(DatabaseError::InteractionFailed);
                },
            }
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn get_summaries_for_last_24_hours(
        &self,
        pool_id: i32,
    ) -> Result<Vec<Summary>, DatabaseError> {
        let sql = r#"
SELECT
        m.pubkey                                      as miner_pubkey,
        COUNT(c.id)::int                              as num_of_contributions,
        MIN(c.difficulty)                             as min_diff,
        ROUND(AVG(c.difficulty)::numeric, 2)::float8  as avg_diff,
        MAX(c.difficulty)                             as max_diff,
        SUM(e.amount)::bigint                         as earning_sub_total,
        ROUND((SUM(e.amount) / SUM(SUM(e.amount)) OVER () * 100)::numeric, 2)::float8 AS percent
    FROM
        contributions c
            INNER JOIN miners m ON c.miner_id = m.id
            INNER JOIN earnings e ON c.challenge_id = e.challenge_id AND c.miner_id = e.miner_id AND e.pool_id = $1
            INNER JOIN pools p ON e.pool_id = p.id
    WHERE
        c.created >= NOW() - INTERVAL '24 hour' AND
        c.created < NOW() AND
        m.enabled = true
    GROUP BY m.pubkey
    ORDER BY percent DESC
        "#;

        if let Ok(db_conn) = self.get_connection().await {
            let stmt = db_conn.prepare_cached(sql).await.unwrap();
            let mut summaries = vec![];
            let summary_iter =
                db_conn.query(&stmt, &[&pool_id]).await.unwrap().into_iter().map(|row| Summary {
                    miner_pubkey: row.get(0),
                    num_of_contributions: row.get(1),
                    min_diff: row.get(2),
                    avg_diff: row.get(3),
                    max_diff: row.get(4),
                    earning_sub_total: row.get(5),
                    percent: row.get(6),
                });

            for summary in summary_iter {
                summaries.push(summary);
            }
            return Ok(summaries);
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }

    #[cfg(feature = "powered-by-dbms-postgres")]
    pub async fn signup_enrollment(
        &self,
        user_pubkey: String,
        pool_authority_pubkey: String,
    ) -> Result<(), DatabaseError> {
        // let sql_insert_miner = r#"INSERT INTO miners (pubkey, enabled) VALUES ($1, $2)"#;
        let sql_insert_miner =
            r#"INSERT INTO miners (pubkey, enabled, status) VALUES ($1, $2, $3)"#;

        let sql_select_miner =
            r#"SELECT id, pubkey, enabled, status FROM miners WHERE miners.pubkey = $1"#;

        let sql_select_pool = r#"SELECT id, proof_pubkey, authority_pubkey, total_rewards, claimed_rewards, pool_pubkey FROM pools WHERE pools.authority_pubkey = $1"#;

        let sql_insert_rewards = r#"INSERT INTO rewards (miner_id, pool_id) VALUES ($1, $2)"#;

        if let Ok(mut db_conn) = self.get_connection().await {
            let stmt_insert_miner = db_conn.prepare_cached(sql_insert_miner).await.unwrap();
            let stmt_select_miner = db_conn.prepare_cached(sql_select_miner).await.unwrap();
            let stmt_select_pool = db_conn.prepare_cached(sql_select_pool).await.unwrap();
            let stmt_insert_rewards = db_conn.prepare_cached(sql_insert_rewards).await.unwrap();

            let mut rows_affected = 0;
            let tx = db_conn.transaction().await.unwrap();

            rows_affected +=
                tx.execute(&stmt_insert_miner, &[&user_pubkey, &true, &"Enrolled"]).await.unwrap();

            let miner =
                tx.query_one(&stmt_select_miner, &[&user_pubkey]).await.map(|row| Miner {
                    id: row.get(0),
                    pubkey: row.get(1),
                    enabled: row.get(2),
                    status: row.get(3),
                })?;

            let pool =
                tx.query_one(&stmt_select_pool, &[&pool_authority_pubkey]).await.map(|row| {
                    models::Pool {
                        id: row.get(0),
                        proof_pubkey: row.get(1),
                        authority_pubkey: row.get(2),
                        total_rewards: row.get(3),
                        claimed_rewards: row.get(4),
                        pool_pubkey: row.get(5),
                    }
                })?;

            rows_affected +=
                tx.execute(&stmt_insert_rewards, &[&miner.id, &pool.id]).await.unwrap();

            tx.commit().await.unwrap();

            info!(target: "server_log", "Records inserted: {}", rows_affected);
            if rows_affected == 0 {
                error!(target: "server_log", "Failed to signup enrollment for pubkey: {}", user_pubkey);
                return Err(DatabaseError::FailedToInsertRow);
            }

            info!(target: "server_log", "Successfully signup enrollment for pubkey: {}", user_pubkey);
            return Ok(());
        } else {
            return Err(DatabaseError::FailedToGetConnectionFromPool);
        }
    }
}
