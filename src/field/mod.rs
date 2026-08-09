//! Finite-field substrate.
//!
//! Public TECA schemes use Lübeck StandardFF-compatible descriptors and the
//! owned `ExplicitField` arithmetic engine.

mod element;
pub mod hasse;
pub mod orders;
pub mod polynomial;
pub mod standardff;

pub use element::{ExplicitField, FieldElement, FieldError};
pub use standardff::{StandardFfError, StandardFieldDescriptor};
