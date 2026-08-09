//! Offline exact cooker primitives.

pub mod actions;
pub mod bounds;
pub mod build;
pub mod certificate;
pub mod matrix;
pub mod objective;
pub mod search;
pub mod tree;

pub use search::{CandidateTest, ExactCookError, ExactCooker, ExactSolution, PairOutcome};

pub use actions::{ActionError, GeneratedActions, generate_actions};
pub use build::{CookBuildError, cook_prior, cook_static};
