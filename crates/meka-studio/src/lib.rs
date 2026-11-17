//! Meka Studio: the web interface for Meka, served from the Meka host. It signs its one user in,
//! passes their requests to Meka on loopback, and adds what Meka leaves to the host: Files, a
//! terminal, push notifications, and Meka's status. The `meka-studio` command runs [`cli`]; the
//! desktop app runs it inside its own process through [`desktop`].

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
compile_error!("Meka Studio runs on Linux, macOS and Windows");

mod api;
mod app;
mod auth;
pub mod cli;
mod config;
pub mod desktop;
mod error;
mod files;
mod git;
mod meka;
mod proxy;
mod push;
mod sse;
mod store;
mod terminal;
mod web;

#[cfg(test)]
mod tests;
