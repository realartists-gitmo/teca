//! Offline prior boundary. Runtime addressing consumes cooked schemes only.

pub mod artifact;
pub mod ctm;

pub use artifact::{
    PairPrior, PriorArtifact, PriorBuilder, PriorMetadata, PriorTransform, ProvenanceRecord,
    Scenario, SymbolSpace, WeightExpr,
};
pub use ctm::{CANONICAL_B9_FILENAME, CtmError, from_fixed_point_scenarios, load_canonical_b9};
