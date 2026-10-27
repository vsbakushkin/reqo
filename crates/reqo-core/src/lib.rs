//! UI-independent part of reqo: the request file format, environments and
//! request execution.

mod env;
mod exec;
mod request;

pub use env::{ENV_FILE, Environments, Variables, substitute};
pub use exec::{Response, execute};
pub use request::{METHODS, Request};
