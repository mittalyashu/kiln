//! In-app scheduler (SPEC §4.9).
//!
//! Owns cron schedules bound to a script and preset, and calls the same
//! `runner::run_script` entry point as manual runs. v1 fires only while the app
//! is running. Implemented alongside the scheduling feature.
