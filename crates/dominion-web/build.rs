//! The commit this client is built from and its time (UTC, `YYYYMMDDHHMMSS`), as
//! `DOMINION_COMMIT` and `DOMINION_COMMIT_TIME`. Nix builds have no `.git` and set both;
//! otherwise git is asked. Empty when neither knows.

include!("../../build/commit.rs");
