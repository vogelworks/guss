//! Shared Postgres persistence: connection config and schema setup.
//!
//! Every task crate can call `db::connect()` to reach the shared Postgres instance.
//! Connection settings live in an untracked `-config/db.toml`, matching the existing
//! `-config/mykey.txt` convention (see `config.rs`, `paths::mykey_file`).
//!
//! No read/write helpers are exposed yet — that surface grows only as a specific
//! subcommand needs it, so callers can't reach data nothing has wired them up to see.

use crate::error::{Result, anyhow};
use crate::paths;
use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Serialize, Deserialize)]
pub struct DbConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub dbname: String,
}

impl DbConfig {
    fn load() -> Result<Self> {
        let path = paths::db_config_file()?;
        if !path.exists() {
            return Err(anyhow!(
                "no db config at {} — create it with host/port/user/password/dbname",
                path.display()
            ));
        }
        let contents = fs::read_to_string(&path)?;
        Ok(toml::from_str(&contents)?)
    }
}

/// Open a connection using the config at `-config/db.toml`. Plaintext (NoTls) —
/// revisit once the server's network exposure is settled.
pub fn connect() -> Result<postgres::Client> {
    let cfg = DbConfig::load()?;
    let conn_str = format!(
        "host={} port={} user={} password={} dbname={}",
        cfg.host, cfg.port, cfg.user, cfg.password, cfg.dbname
    );
    Ok(postgres::Client::connect(&conn_str, postgres::NoTls)?)
}

/// Create the tables if they don't exist yet. Provisional schema — expect this to
/// change as the exact tracked fields get nailed down.
const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS builds (
    id SERIAL PRIMARY KEY,
    cluster TEXT NOT NULL,
    service TEXT NOT NULL,
    tag TEXT NOT NULL,
    version TEXT NOT NULL,
    built_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS current_versions (
    cluster TEXT NOT NULL,
    service TEXT NOT NULL,
    tag TEXT NOT NULL,
    version TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (cluster, service, tag)
);

CREATE TABLE IF NOT EXISTS health (
    id SERIAL PRIMARY KEY,
    cluster TEXT NOT NULL,
    service TEXT NOT NULL,
    status TEXT NOT NULL,
    detail TEXT,
    checked_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
";

pub fn init_schema(client: &mut postgres::Client) -> Result<()> {
    client.batch_execute(SCHEMA_SQL)?;
    Ok(())
}
