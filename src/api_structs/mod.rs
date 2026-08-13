//! Serde deserialization structs for the shared 3cat media API responses.

use std::fmt::Display;

use serde::Deserialize;

/// Placeholder error type returned by the 3cat API on failure.
#[derive(Debug, Deserialize)]
pub struct Tv3Error {}

impl Display for Tv3Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Error")
    }
}
