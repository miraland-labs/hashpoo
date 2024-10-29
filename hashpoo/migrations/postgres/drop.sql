DROP TRIGGER IF EXISTS update_timestamp_trigger ON init_completion CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON earnings CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON rewards CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON claims CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON transactions CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON submissions CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON contributions CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON challenges CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON pools CASCADE;
DROP TRIGGER IF EXISTS update_timestamp_trigger ON miners CASCADE;

DROP FUNCTION IF EXISTS update_timestamp() CASCADE;

DROP TABLE IF EXISTS init_completion;
DROP TABLE IF EXISTS earnings;
DROP TABLE IF EXISTS rewards;
DROP TABLE IF EXISTS claims;
DROP TABLE IF EXISTS transactions;
DROP TABLE IF EXISTS submissions;
DROP TABLE IF EXISTS contributions;
DROP TABLE IF EXISTS challenges;
DROP TABLE IF EXISTS pools;
DROP TABLE IF EXISTS miners;
