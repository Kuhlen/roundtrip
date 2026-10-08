//! Roundtrip domain: request/response types, collection tree, environments, history, interpolation. No IO.

pub mod auth;
pub mod collection;
pub mod environment;
pub mod error;
pub mod graphql;
pub mod history;
pub mod http;
pub mod interpolation;
pub mod session;

pub use error::AppError;
