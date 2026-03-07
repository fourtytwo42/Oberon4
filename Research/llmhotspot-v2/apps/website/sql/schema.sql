-- Website-side state tables (Postgres)

CREATE TABLE IF NOT EXISTS web_customer_sessions (
  session_id TEXT PRIMARY KEY,
  email TEXT NOT NULL,
  challenge_id TEXT NULL,
  created_unix BIGINT NOT NULL,
  expires_unix BIGINT NOT NULL,
  last_context_json TEXT NULL
);

CREATE INDEX IF NOT EXISTS idx_web_customer_sessions_expires
  ON web_customer_sessions(expires_unix);

CREATE TABLE IF NOT EXISTS web_admin_sessions (
  session_id TEXT PRIMARY KEY,
  broker_session_id TEXT NOT NULL,
  created_unix BIGINT NOT NULL,
  expires_unix BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_web_admin_sessions_expires
  ON web_admin_sessions(expires_unix);

CREATE TABLE IF NOT EXISTS web_idempotency_keys (
  idem_key TEXT PRIMARY KEY,
  route_key TEXT NOT NULL,
  response_json TEXT NOT NULL,
  created_unix BIGINT NOT NULL,
  expires_unix BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS web_contact_requests (
  request_id TEXT PRIMARY KEY,
  email TEXT NOT NULL,
  company TEXT NULL,
  requested_seats BIGINT NOT NULL,
  note TEXT NULL,
  created_unix BIGINT NOT NULL,
  status TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS web_customer_access_tokens (
  access_token TEXT PRIMARY KEY,
  email TEXT NOT NULL UNIQUE,
  created_unix BIGINT NOT NULL,
  last_used_unix BIGINT NULL,
  consumed_unix BIGINT NULL,
  revoked_unix BIGINT NULL
);

CREATE INDEX IF NOT EXISTS idx_web_customer_access_tokens_email
  ON web_customer_access_tokens(email);

CREATE TABLE IF NOT EXISTS web_smtp_settings (
  config_id TEXT PRIMARY KEY,
  host TEXT NOT NULL,
  port INTEGER NOT NULL,
  secure BOOLEAN NOT NULL,
  username TEXT NOT NULL,
  password TEXT NOT NULL,
  from_email TEXT NOT NULL,
  from_name TEXT NULL,
  updated_unix BIGINT NOT NULL
);
