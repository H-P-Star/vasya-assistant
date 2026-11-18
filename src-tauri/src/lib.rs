pub mod activation;
pub mod browser;
pub mod calibration;
pub mod commands;
pub mod config;
pub mod credentials;
pub mod generator;
pub mod runtime;
pub mod speech;
pub mod voice;
pub mod web_search;

#[cfg(windows)]
pub mod audio;
#[cfg(all(windows, feature = "desktop"))]
pub mod desktop;
#[cfg(all(windows, feature = "desktop"))]
pub mod island;
#[cfg(windows)]
pub mod platform;
