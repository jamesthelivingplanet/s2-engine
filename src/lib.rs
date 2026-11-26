//! S2 Engine: run language models on this machine, one at a time.

pub mod cli;
pub mod config;
pub mod hermes;
pub mod hooked;
pub mod manage;
pub mod paths;
pub mod server;
pub mod skill;
pub mod talk;
pub mod tray;

/// Every failure is a message for the person at the terminal.
pub type Result<T> = std::result::Result<T, String>;
