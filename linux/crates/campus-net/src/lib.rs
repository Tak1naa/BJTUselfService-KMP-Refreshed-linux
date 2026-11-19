pub mod auth;
pub mod fetch;
#[cfg(feature = "media")]
pub mod media;
pub mod phyvlab;
pub mod transport;
pub use transport::{Endpoints, SchoolTransport};

mod legacy_homework;
#[cfg(test)]
mod tests;
