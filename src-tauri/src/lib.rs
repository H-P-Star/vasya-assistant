pub mod activation;
pub mod browser;
pub mod commands;
pub mod config;
pub mod generator;
pub mod runtime;
pub mod speech;

#[cfg(windows)]
pub mod audio;
#[cfg(all(windows, feature = "desktop"))]
pub mod desktop;
#[cfg(windows)]
pub mod platform;
