//! UI-independent part of reqo: the request file format, environments and
//! request execution.

mod env;
mod request;

pub use env::{ENV_FILE, Environments, Variables, substitute};
pub use request::{METHODS, Request};
