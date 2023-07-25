// Copyright 2022 Oxide Computer Company

pub mod cli;
pub mod config;
pub mod mgmt;
pub mod p9;

// Re-export p4rs so consumers can rely on matching types
pub use p4rs;
