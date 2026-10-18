//! Configuration, read once from the environment at start-up.
//!
//! Two rules this module enforces:
//!
//! * **Secrets are never defaulted.** A default secret is a secret in production.
//! * **Half-configured is a boot failure.** A subsystem that is partly set up fails now, with a
//!   sentence saying what is missing, rather than at three in the morning with a stack trace.

use std::env;

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{0} is not set")]
    Missing(&'static str),
    #[error("{name} is not a number: {value}")]
    NotANumber { name: &'static str, value: String },
}

fn var(name: &'static str) -> Result<String, ConfigError> {
    match env::var(name) {
        Ok(v) if !v.trim().is_empty() => Ok(v),
        _ => Err(ConfigError::Missing(name)),
    }
}

fn var_or(name: &str, default: &str) -> String {
    env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn num<T: std::str::FromStr>(name: &'static str, default: &str) -> Result<T, ConfigError> {
    parse_num(name, &var_or(name, default))
}

/// Split out from `num` so it can be tested without mutating the process environment.
///
/// Mutating `env` in a test is `unsafe` in this edition, and it is shared state across threads —
/// two tests setting the same variable race. A pure function has neither problem.
fn parse_num<T: std::str::FromStr>(name: &'static str, raw: &str) -> Result<T, ConfigError> {
    raw.trim().parse().map_err(|_| ConfigError::NotANumber {
        name,
        value: raw.to_string(),
    })
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub database_url: String,
    pub bind: String,
    pub allowed_origins: Vec<String>,

    pub network_passphrase: String,
    pub rpc_url: String,
    pub horizon_url: String,

    /// The deployed registry contract.
    pub registry_contract_id: String,
    /// The G-account payments are drawn on.
    pub treasury_address: String,

    /// The account that CREATES payment requests, and the only party the contract lets cancel one.
    ///
    /// Deliberately **not** the treasury. `create_request` calls `creator.require_auth()`, so the
    /// creator must be an account this service can authorise as — and the treasury is precisely the
    /// account that cannot sign alone, by design. Pointing the creator at it makes every request
    /// creation need two of three signatures, which is not what anybody meant by "record that we owe
    /// this vendor".
    pub business_address: String,

    /// Stellar CLI key names. The CLI holds the secrets in its own keystore, so no private key
    /// ever passes through this process or its environment.
    ///
    /// `business_key` must be the key for `business_address`, or `require_auth` refuses.
    pub business_key: String,
    pub settler_key: String,

    /// The USDC issuer on this network.
    pub usdc_issuer: String,
    pub usdc_contract: String,
    /// The Stellar Asset Contract for **native XLM**.
    ///
    /// Native still has one. The registry's `asset` field is an Address, and there is no address
    /// meaning "no asset" — leaving it empty made `create_request` fail on a missing argument, so
    /// every XLM payment raised through the interface died before it reached the chain.
    pub xlm_contract: String,

    pub session_key: Vec<u8>,
    pub home_domain: String,
    pub challenge_ttl_seconds: i64,

    pub tx_timebounds_seconds: u64,
    pub base_fee: u32,

    pub indexer_interval_seconds: u64,
    pub submitter_interval_seconds: u64,
    pub sweep_interval_seconds: u64,
}

impl Settings {
    pub fn from_env() -> Result<Self, ConfigError> {
        Ok(Self {
            database_url: var("DATABASE_URL")?,
            bind: var_or("BIND", "0.0.0.0:8080"),
            allowed_origins: var_or("ALLOWED_ORIGINS", "http://localhost:5173")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),

            network_passphrase: var_or("NETWORK_PASSPHRASE", settle_chain::TESTNET),
            rpc_url: var_or("SOROBAN_RPC_URL", "https://soroban-testnet.stellar.org"),
            horizon_url: var_or("HORIZON_URL", "https://horizon-testnet.stellar.org"),

            registry_contract_id: var("REGISTRY_CONTRACT_ID")?,
            treasury_address: var("TREASURY_ADDRESS")?,

            business_address: var("BUSINESS_ADDRESS")?,
            business_key: var("BUSINESS_KEY")?,
            settler_key: var("SETTLER_KEY")?,

            usdc_issuer: var("USDC_ISSUER")?,
            usdc_contract: var_or("USDC_CONTRACT", ""),
            // Deterministic per network, so the testnet value is a sane default rather than one
            // more thing to set before anything works.
            xlm_contract: var_or(
                "XLM_CONTRACT",
                "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
            ),

            // Never defaulted.
            session_key: var("SESSION_KEY")?.into_bytes(),
            home_domain: var_or("HOME_DOMAIN", "localhost:5173"),
            challenge_ttl_seconds: num("CHALLENGE_TTL_SECONDS", "300")?,

            tx_timebounds_seconds: num("TX_TIMEBOUNDS_SECONDS", "3600")?,
            base_fee: num("BASE_FEE", "1000")?,

            indexer_interval_seconds: num("INDEXER_INTERVAL_SECONDS", "5")?,
            submitter_interval_seconds: num("SUBMITTER_INTERVAL_SECONDS", "3")?,
            sweep_interval_seconds: num("SWEEP_INTERVAL_SECONDS", "60")?,
        })
    }

    pub fn network(&self) -> settle_chain::Network {
        settle_chain::Network::new(self.network_passphrase.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A guard against somebody helpfully giving the session key a default.
    ///
    /// This reads the source of this very file. If the line that requires `SESSION_KEY` ever
    /// becomes a `var_or`, this test fails and says why.
    #[test]
    fn secrets_are_never_defaulted() {
        let source = include_str!("config.rs");
        assert!(
            source.contains(r#"var("SESSION_KEY")?"#),
            "SESSION_KEY must be required, never defaulted — a default secret is a secret in production"
        );
        for required in [
            "REGISTRY_CONTRACT_ID",
            "TREASURY_ADDRESS",
            "BUSINESS_ADDRESS",
            "BUSINESS_KEY",
            "SETTLER_KEY",
        ] {
            assert!(
                source.contains(&format!(r#"var("{required}")?"#)),
                "{required} must be required, not defaulted"
            );
        }
    }

    #[test]
    fn a_missing_required_variable_names_itself() {
        let e = var("SETTLEFLOW_DEFINITELY_NOT_SET").unwrap_err();
        assert_eq!(e.to_string(), "SETTLEFLOW_DEFINITELY_NOT_SET is not set");
    }

    #[test]
    fn an_unset_variable_falls_back_to_its_default() {
        assert_eq!(
            var_or("SETTLEFLOW_DEFINITELY_NOT_SET", "fallback"),
            "fallback"
        );
    }

    #[test]
    fn a_non_numeric_value_reports_what_it_saw() {
        let e = parse_num::<u64>("INTERVAL", "not-a-number").unwrap_err();
        assert!(e.to_string().contains("not-a-number"));
        assert!(e.to_string().contains("INTERVAL"));
    }

    #[test]
    fn a_numeric_value_with_whitespace_still_parses() {
        // Values pasted into a compose file routinely carry a trailing space.
        assert_eq!(parse_num::<u64>("INTERVAL", " 30 ").unwrap(), 30);
    }
}
