//! OS keychain secret storage (SPEC §4.6).
//!
//! Wraps the `keyring` crate. Secret values are injected into runs as
//! environment variables and must never touch git, the database, or logs.
//! Implemented alongside the secrets feature.
