use std::fmt;

use num_bigint::BigUint;

use crate::bundle::{BundleError, ProbeBundle};
use crate::prior::{PairPrior, WeightExpr};

use super::search::{CandidateTest, PairOutcome};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MatrixError {
    SymbolCardinalityTooSmall,
    Bundle(BundleError),
    MixedFixedPointScales,
}

impl fmt::Display for MatrixError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SymbolCardinalityTooSmall => {
                write!(f, "prior symbol space must contain at least two symbols")
            }
            Self::Bundle(err) => err.fmt(f),
            Self::MixedFixedPointScales => {
                write!(f, "fixed-point scenario weights use inconsistent scales")
            }
        }
    }
}

impl std::error::Error for MatrixError {}

impl From<BundleError> for MatrixError {
    fn from(value: BundleError) -> Self {
        Self::Bundle(value)
    }
}

/// Precompute bundle outcomes independently of the prior weight representation.
pub fn pair_outcome_matrix(
    prior: &impl PairPrior,
    bundles: &[ProbeBundle],
    capacity: u32,
) -> Result<Vec<CandidateTest>, MatrixError> {
    let symbol_cardinality = prior.symbol_space().cardinality;
    if symbol_cardinality < 2 {
        return Err(MatrixError::SymbolCardinalityTooSmall);
    }
    let mut tests = Vec::with_capacity(bundles.len());
    for (id, bundle) in bundles.iter().enumerate() {
        let mut outcomes = Vec::with_capacity(prior.scenarios().len());
        for scenario in prior.scenarios() {
            let left = bundle
                .evaluate_symbols(&scenario.left, symbol_cardinality, capacity)?
                .0;
            let right = bundle
                .evaluate_symbols(&scenario.right, symbol_cardinality, capacity)?
                .0;
            outcomes.push(PairOutcome { left, right });
        }
        tests.push(CandidateTest {
            id: id as u32,
            outcomes,
        });
    }
    Ok(tests)
}

pub fn fixed_point_matrix(
    prior: &impl PairPrior,
    bundles: &[ProbeBundle],
    capacity: u32,
) -> Result<(Vec<BigUint>, Vec<CandidateTest>, u32), MatrixError> {
    let tests = pair_outcome_matrix(prior, bundles, capacity)?;
    let mut scale = None;
    let mut weights = Vec::with_capacity(prior.scenarios().len());
    for scenario in prior.scenarios() {
        match &scenario.weight {
            WeightExpr::FixedPoint {
                units,
                fractional_bits,
            } => {
                if scale.is_some_and(|s| s != *fractional_bits) {
                    return Err(MatrixError::MixedFixedPointScales);
                }
                scale = Some(*fractional_bits);
                weights.push(units.clone());
            }
        }
    }
    Ok((weights, tests, scale.unwrap_or(0)))
}
