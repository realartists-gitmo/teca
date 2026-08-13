//! TECA — Token-Efficient Content Addressing.
//!
//! The runtime semantic boundary is deliberately small:
//! caller bytes -> cooked `Scheme` -> lazy structural `AtomId` stream.
//! Priors and cookers are offline concerns and are not runtime dependencies.
//!
//! The low-level [`Address`] API is stateless: it maps bytes to an indefinitely
//! extensible stream of structural atom IDs. Without external bookkeeping it
//! cannot know when two streams collide at a given prefix. [`Neighborhood`] is
//! the canonical stateful namespace layer above that boundary: it owns
//! shortest-unique address assignment, collision detection and extension,
//! canonical shortening after removal, and prefix resolution over a changing
//! set of co-occurring contents. Identifiers are opaque downstream bytes and
//! never affect address generation.

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
pub mod neighborhood;
pub mod prior;
pub mod probe;
pub mod render;
pub mod scheme;

pub use address::{Address, AtomId, TokenAddress, TokenAddressError};
pub use artifact::{
    ArtifactError, CookCertificateArtifact, CookObjectiveDescriptor, CookStatus, LexiconArtifact,
    SchemeArtifact, decode_lexicon, decode_neighborhood, decode_prior, decode_scheme,
    encode_lexicon, encode_neighborhood, encode_prior, encode_scheme,
};
pub use bundle::ProbeBundle;
pub use codec::ContentCodecDescriptor;
pub use defaults::{default_address, default_atom_ids, default_lexicon, default_scheme};
pub use fallback::FallbackDescriptor;
pub use lexicon::{Lexicon, LexiconError};
pub use neighborhood::{
    AddressChange, InsertResult, InsertStatus, Neighborhood, NeighborhoodAddress,
    NeighborhoodEntry, NeighborhoodError, NeighborhoodRow, RemoveResult, Resolution,
};
pub use render::{render_neighborhood_address, render_text_prefix};
pub use scheme::{DecisionNode, Scheme, SchemeError};
