//! The single owner of run logic (SPEC §6.1-6.4).
//!
//! `run_script(script, values, trigger)` will resolve cwd and env, inject
//! parameters and secrets, enforce strict mode, spawn the process group,
//! stream logs, record history, and notify. Manual runs, re-runs, and
//! schedules all enter through that same function. Implemented alongside the
//! execution engine feature.
