/*
    db initialization
     sudo -i -u postgres
     psql
     or
    sudo -i -u postgres psql

    CREATE ROLE miracle LOGIN CREATEDB CREATEROLE;
    ALTER USER miracle WITH PASSWORD 'Mirascape';
    psql postgres -U miracle;
    OR
    psql -U miracle -h localhost -d postgres

    CREATE DATABASE miraland;
    or
    createdb miraland --owner miracle

    up.sql
    GRANT ALL ON SCHEMA ore TO <other_user>;

    DATABASE_URL=postgres://username:password@localhost/mydb
*/

-- CREATE DATABASE miraland;
/*
DROP SCHEMA IF EXISTS poolore;
CREATE SCHEMA IF NOT EXISTS poolore AUTHORIZATION miracle;
*/

BEGIN

DROP TRIGGER IF EXISTS update_timestamp_trigger ON miners CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON pools CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON challenges CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON submissions CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON contributions CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON transactyions CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON claims CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON rewards CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON earnings CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON init_completion CASCADE;

DROP FUNCTION IF EXISTS update_timestamp();

DROP TABLE IF EXISTS miners;
DROP TABLE IF EXISTS pools;
DROP TABLE IF EXISTS challenges;
DROP TABLE IF EXISTS submissions;
DROP TABLE IF EXISTS contributions;
DROP TABLE IF EXISTS transactions;
DROP TABLE IF EXISTS claims;
DROP TABLE IF EXISTS rewards;
DROP TABLE IF EXISTS earnings;
DROP TABLE IF EXISTS init_completion;

/*
    common utilities like triggers, procdures, etc.
*/
CREATE OR REPLACE FUNCTION update_timestamp()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated := CURRENT_TIMESTAMP;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Templates
-- CREATE TRIGGER update_timestamp_trigger
-- BEFORE UPDATE ON my_table
-- FOR EACH ROW
-- EXECUTE FUNCTION update_timestamp();


/*
    id - primary key, auto created index
    PostgreSQL automatically creates a unique index when a unique constraint or primary key is defined for a table.
    user index naming convention(with prefix):
    pkey_: primary key
    ukey_: user key
    fkey_: foreign key
    uniq_: unique index
    indx_: other index (multiple values)

    status: Enrolled, Activated, Frozen, Deactivated
*/

CREATE TABLE miners (
    id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY
        (START WITH 1000 INCREMENT BY 1),
    pubkey VARCHAR(44) NOT NULL,
    enabled BOOLEAN DEFAULT false NOT NULL,
    status VARCHAR(30) DEFAULT 'Enrolled' NOT NULL,
    created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
);

CREATE OR REPLACE TRIGGER update_timestamp_trigger
BEFORE UPDATE ON miners
FOR EACH ROW
EXECUTE FUNCTION update_timestamp();

CREATE UNIQUE INDEX uniq_miners_pubkey ON miners (pubkey ASC);


/*
    id - primary key, auto created index
    PostgreSQL automatically creates a unique index when a unique constraint or primary key is defined for a table.
    user index naming convention(with prefix):
    pkey_: primary key
    ukey_: user key
    fkey_: foreign key
    uniq_: unique index
    indx_: other index (multiple values)

    status: Enrolled, Activated, Frozen, Deactivated
*/
-- CREATE TABLE members (
--     id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY
--         (START WITH 1000 INCREMENT BY 1),
--     pubkey VARCHAR(44) NOT NULL,
--     enabled BOOLEAN DEFAULT false NOT NULL,
--     role_miner BOOLEAN DEFAULT false NOT NULL,
--     role_operator BOOLEAN DEFAULT false NOT NULL,
--     status VARCHAR(30) DEFAULT 'Enrolled' NOT NULL,
--     created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
--     updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
-- );

-- CREATE OR REPLACE TRIGGER update_timestamp_trigger
-- BEFORE UPDATE ON members
-- FOR EACH ROW
-- EXECUTE FUNCTION update_timestamp();

-- CREATE UNIQUE INDEX uniq_members_pubkey ON members (pubkey ASC);


/*
    id - primary key, auto created index
    PostgreSQL automatically creates a unique index when a unique constraint or primary key is defined for a table.
    user index naming convention(with prefix):
    pkey_: primary key
    ukey_: user key
    fkey_: foreign key
    uniq_: unique index
    indx_: other index (multiple values)

    status: Enrolled, Activated, Frozen, Deactivated
    address: member_pda(authority, pool_address)
    authority: miner/client/contributor's wallet public address
    pool_address: pool_pda(authority, ore_pool_api::id())
    total_balance: accrued rewards
*/
CREATE TABLE members (
    address VARCHAR PRIMARY KEY,
    id BIGINT NOT NULL,
    authority VARCHAR NOT NULL,
    pool_address VARCHAR NOT NULL,
    total_balance BIGINT NOT NULL,
    is_approved BOOLEAN NOT NULL,
    is_kyc BOOLEAN NOT NULL,
    is_synced BOOLEAN NOT NULL,
    is_operator BOOLEAN DEFAULT false NOT NULL,
    status VARCHAR(30) DEFAULT 'Enrolled' NOT NULL,
    created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
);

CREATE OR REPLACE TRIGGER update_timestamp_trigger
BEFORE UPDATE ON members
FOR EACH ROW
EXECUTE FUNCTION update_timestamp();

-- primary key index implicitly created
-- CREATE UNIQUE INDEX uniq_members_address ON members (address ASC);
CREATE UNIQUE INDEX uniq_members_id ON members (id ASC);
CREATE UNIQUE INDEX uniq_members_authority_pool_address ON members (authority ASC, pool_address ASC);
CREATE INDEX indx_pools_authority_pubkey ON pools (authority_pubkey ASC);


/*
    pool_pubkey: pool_pda(authority, ore_pool_api::id()), pool pda account address
    proof_pubkey: Public Pool: pool_proof_pda(pool_pda, ore_api::id()). Private Pool: proof_pda(authority, ore_api::id())
    authority_pubkey: pool(either public or private/solo) operator's wallet public address
    total_balance: accrued rewards
*/
CREATE TABLE pools (
    id INT PRIMARY KEY GENERATED ALWAYS AS IDENTITY
        (START WITH 1000 INCREMENT BY 1),
    pool_pubkey VARCHAR(44) NOT NULL,
    proof_pubkey VARCHAR(44) NOT NULL,
    authority_pubkey VARCHAR(44) NOT NULL,
    total_rewards BIGINT DEFAULT 0 NOT NULL,
    claimed_rewards BIGINT DEFAULT 0 NOT NULL,
    created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
);

CREATE OR REPLACE TRIGGER update_timestamp_trigger
BEFORE UPDATE ON pools
FOR EACH ROW
EXECUTE FUNCTION update_timestamp();

CREATE UNIQUE INDEX indx_pools_pool_pubkey ON pools (pool_pubkey ASC);
CREATE UNIQUE INDEX indx_pools_proof_pubkey ON pools (proof_pubkey ASC);
CREATE UNIQUE INDEX indx_pools_authority_pubkey ON pools (authority_pubkey ASC);


CREATE TABLE challenges (
    id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    pool_id INT NOT NULL,
    contribution_id BIGINT,
    challenge BYTEA NOT NULL,
    rewards_earned BIGINT DEFAULT 0,
    created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
);

CREATE OR REPLACE TRIGGER update_timestamp_trigger
BEFORE UPDATE ON challenges
FOR EACH ROW
EXECUTE FUNCTION update_timestamp();

CREATE INDEX indx_challenges_challenge ON challenges (challenge ASC);


CREATE TABLE submissions (
  id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
  miner_id BIGINT NOT NULL,
  challenge_id BIGINT NOT NULL,
  difficulty SMALLINT NOT NULL,
  nonce BIGINT NOT NULL,
  digest BYTEA,
  created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
  updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
);

CREATE OR REPLACE TRIGGER update_timestamp_trigger
BEFORE UPDATE ON submissions
FOR EACH ROW
EXECUTE FUNCTION update_timestamp();

CREATE INDEX indx_submissions_miner_challenge_ids ON submissions (miner_id ASC, challenge_id ASC);
CREATE INDEX indx_submissions_nonce ON submissions (nonce ASC);


CREATE TABLE contributions (
  id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
  miner_id BIGINT NOT NULL,
  challenge_id BIGINT NOT NULL,
  difficulty SMALLINT NOT NULL,
  nonce BIGINT NOT NULL,
  digest BYTEA,
  created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
  updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
);

CREATE OR REPLACE TRIGGER update_timestamp_trigger
BEFORE UPDATE ON contributions
FOR EACH ROW
EXECUTE FUNCTION update_timestamp();

CREATE INDEX indx_contributions_miner_challenge_ids ON contributions (miner_id ASC, challenge_id ASC);
CREATE INDEX indx_contributions_nonce ON contributions (nonce ASC);
CREATE INDEX indx_contributions_created ON contributions (created DESC);
CREATE INDEX indx_contributions_challenge_id ON contributions (challenge_id ASC);


CREATE TABLE transactions (
  id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
  transaction_type VARCHAR(30) NOT NULL,
  signature VARCHAR(200) NOT NULL,
  priority_fee INT DEFAULT 0 NOT NULL,
  pool_id INT,
  miner_id BIGINT,
  created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
  updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
);

CREATE OR REPLACE TRIGGER update_timestamp_trigger
BEFORE UPDATE ON transactions
FOR EACH ROW
EXECUTE FUNCTION update_timestamp();

CREATE INDEX indx_transactions_signature ON transactions (signature ASC);
CREATE INDEX indx_transactions_pool_id_created ON transactions (pool_id ASC, transaction_type ASC, created DESC);
CREATE INDEX indx_transactions_miner_id_created ON transactions (miner_id ASC, created DESC);


CREATE TABLE claims (
  id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
  miner_id BIGINT NOT NULL,
  pool_id INT NOT NULL,
  transaction_id BIGINT NOT NULL,
  amount BIGINT NOT NULL,
  created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
  updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
);

CREATE OR REPLACE TRIGGER update_timestamp_trigger
BEFORE UPDATE ON claims
FOR EACH ROW
EXECUTE FUNCTION update_timestamp();

CREATE INDEX indx_claims_miner_pool_txn_ids ON claims (miner_id ASC, pool_id ASC, transaction_id ASC);


CREATE TABLE rewards (
  id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
  miner_id BIGINT NOT NULL,
  pool_id INT NOT NULL,
  balance BIGINT DEFAULT 0 NOT NULL,
  created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
  updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
);

CREATE OR REPLACE TRIGGER update_timestamp_trigger
BEFORE UPDATE ON rewards
FOR EACH ROW
EXECUTE FUNCTION update_timestamp();

CREATE INDEX indx_rewards_miner_pool_ids ON rewards (miner_id ASC, pool_id ASC);


CREATE TABLE earnings (
  id BIGINT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
  miner_id BIGINT NOT NULL,
  pool_id INT NOT NULL,
  challenge_id BIGINT NOT NULL,
  amount BIGINT DEFAULT 0 NOT NULL,
  created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
  updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
);

CREATE OR REPLACE TRIGGER update_timestamp_trigger
BEFORE UPDATE ON earnings
FOR EACH ROW
EXECUTE FUNCTION update_timestamp();

CREATE INDEX indx_earnings_miner_pool_challenge_ids ON earnings (miner_id ASC, pool_id ASC, challenge_id ASC);


CREATE TABLE init_completion (
    id INT PRIMARY KEY GENERATED ALWAYS AS IDENTITY,
    init_completed BOOLEAN DEFAULT false NOT NULL,
    created TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    updated TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
);

CREATE OR REPLACE TRIGGER update_timestamp_trigger
BEFORE UPDATE ON init_completion
FOR EACH ROW
EXECUTE FUNCTION update_timestamp();

INSERT INTO init_completion (init_completed) VALUES (true);


END;

