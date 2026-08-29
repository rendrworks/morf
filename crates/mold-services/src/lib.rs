//! Native system services for mold.

mod pam;

pub use pam::{PamAuthenticator, PamError};
