//! Login-shell environment capture (SPEC §4.7).
//!
//! GUI apps on macOS do not inherit the terminal environment, so startup will
//! capture it once from the user's login shell and cache it for runs. The
//! `PATH` override setting is applied here. Implemented alongside the runner.
