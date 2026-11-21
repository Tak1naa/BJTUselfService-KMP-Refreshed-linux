pub mod auth;
pub mod fetch;
pub mod mail;
#[cfg(feature = "media")]
pub mod media;
pub mod phyvlab;
pub mod transport;
pub mod update;
pub use transport::{Endpoints, SchoolTransport};

mod gbk;
mod legacy_homework;
#[cfg(test)]
mod tests;
