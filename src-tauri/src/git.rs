//! System `git` integration (SPEC §4.3).
//!
//! Shells out to the user's git binary for status, commit, pull, and push.
//! No libgit2 and no credential management of our own. Implemented alongside
//! the repo setup and git sync features.
