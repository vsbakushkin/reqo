//! UI-independent part of reqo: the request file format, environments,
//! workspaces and request execution.

pub mod collection;
mod env;
mod exec;
mod request;
pub mod workspace;

pub use env::{ENV_FILE, Environments, Variables, substitute};
pub use exec::{Response, execute};
pub use request::{METHODS, Request};
