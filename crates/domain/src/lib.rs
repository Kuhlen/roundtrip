//! Roundtrip domain: request/response types, collection tree, environments, interpolation. No IO.

pub mod auth;
pub mod collection;
pub mod environment;
pub mod error;
pub mod http;
pub mod interpolation;
pub mod session;

pub use error::AppError;
