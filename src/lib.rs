//! TECA — Token-Efficient Content Addressing.
//!
//! The implementation follows the canonical binary artifact architecture.
//!
//! The runtime semantic boundary is deliberately small:
//! caller bytes -> cooked `Scheme` -> lazy structural `AtomId` stream.
//! Priors and cookers are offline concerns and are not runtime dependencies.

#![forbid(unsafe_code)]

pub mod address;
pub mod artifact;
pub mod bundle;
pub mod codec;
pub mod cook;
pub mod defaults;
pub mod fallback;
pub mod field;
pub mod lexicon;
pub mod prior;
pub mod probe;
pub mod render;
pub mod scheme;

pub use address::{Address, AtomId, TokenAddress, TokenAddressError};
pub use artifact::{
    ArtifactError, CookCertificateArtifact, CookObjectiveDescriptor, CookStatus, LexiconArtifact,
    SchemeArtifact, decode_lexicon, decode_prior, decode_scheme, encode_lexicon, encode_prior,
    encode_scheme,
};
pub use bundle::ProbeBundle;
pub use codec::ContentCodecDescriptor;
pub use defaults::{default_address, default_atom_ids, default_lexicon, default_scheme};
pub use fallback::FallbackDescriptor;
pub use lexicon::{Lexicon, LexiconError};
pub use scheme::{DecisionNode, Scheme, SchemeError};
