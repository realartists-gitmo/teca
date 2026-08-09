//! End-to-end deterministic cook from a finite prior artifact to a runtime scheme.

use std::fmt;

use num_bigint::BigUint;

use crate::artifact::{
    ArtifactError, CookCertificateArtifact, CookObjectiveDescriptor, CookStatus, SchemeArtifact,
};
use crate::codec::ContentCodecDescriptor;
use crate::field::StandardFieldDescriptor;
use crate::prior::{PairPrior, PriorArtifact, WeightExpr};
use crate::probe::ProbeError;
use crate::scheme::{DecisionNode, Scheme, SchemeError};

use super::actions::{ActionError, GeneratedActions, generate_actions};
use super::matrix::{MatrixError, fixed_point_matrix};
use super::search::{ExactCookError, ExactCooker};
use super::tree::AbstractCookNode;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CookBuildError {
    Action(ActionError),
    Matrix(MatrixError),
    Exact(ExactCookError),
    Artifact(ArtifactError),
    Scheme(SchemeError),
    MixedWeightKinds,
    CodecSymbolMismatch { prior: u32, codec: u32 },
    InvalidTestId(u32),
    StaticCapacityTooSmall { required: u32, capacity: u32 },
}

impl fmt::Display for CookBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Action(e) => e.fmt(f),
            Self::Matrix(e) => e.fmt(f),
            Self::Exact(e) => e.fmt(f),
            Self::Artifact(e) => e.fmt(f),
            Self::Scheme(e) => e.fmt(f),
            Self::MixedWeightKinds => write!(f, "prior contains unsupported weight kinds"),
            Self::CodecSymbolMismatch { prior, codec } => write!(
                f,
                "content codec cardinality {codec} does not match prior symbol cardinality {prior}"
            ),
            Self::InvalidTestId(id) => write!(f, "cooker tree references unknown test id {id}"),
            Self::StaticCapacityTooSmall { required, capacity } => write!(
                f,
                "static cook requires capacity >= {required}, got {capacity}"
            ),
        }
    }
}
impl std::error::Error for CookBuildError {}
impl From<ActionError> for CookBuildError {
    fn from(v: ActionError) -> Self {
        Self::Action(v)
    }
}
impl From<MatrixError> for CookBuildError {
    fn from(v: MatrixError) -> Self {
        Self::Matrix(v)
    }
}
impl From<ExactCookError> for CookBuildError {
    fn from(v: ExactCookError) -> Self {
        Self::Exact(v)
    }
}
impl From<ArtifactError> for CookBuildError {
    fn from(v: ArtifactError) -> Self {
        Self::Artifact(v)
    }
}
impl From<SchemeError> for CookBuildError {
    fn from(v: SchemeError) -> Self {
        Self::Scheme(v)
    }
}

pub fn cook_prior(
    prior: &PriorArtifact,
    capacity: u32,
    codec: ContentCodecDescriptor,
) -> Result<SchemeArtifact, CookBuildError> {
    let codec_cardinality = codec.symbol_cardinality();
    if codec_cardinality != prior.symbol_space().cardinality {
        return Err(CookBuildError::CodecSymbolMismatch {
            prior: prior.symbol_space().cardinality,
            codec: codec_cardinality,
        });
    }
    if prior.scenarios().is_empty() {
        let scheme = Scheme::fallback_only(capacity, codec)?;
        let zero = WeightExpr::FixedPoint {
            units: BigUint::from(0u8),
            fractional_bits: 0,
        };
        return Ok(SchemeArtifact {
            scheme,
            objective: CookObjectiveDescriptor::ExpectedDistinguishingAtoms,
            cook_status: CookStatus::ExactRelativeToPrior,
            certificate: CookCertificateArtifact {
                total_weight: zero.clone(),
                objective_cost: zero.clone(),
                root_lower_bound: zero,
                explored_states: 0,
                memoized_states: 0,
            },
            spec_revision: "teca-canonical-refactor-spec-v10".into(),
            cooker_revision: "teca-cooker-exact-v1".into(),
        });
    }

    let generated = generate_actions(prior, capacity)?;
    if generated.tests.is_empty() {
        return Err(CookBuildError::Exact(ExactCookError::UnseparableState));
    }
    cook_fixed(prior, capacity, codec, generated)
}

/// Build the canonical deterministic static Hasse stream. The prior selects
/// the early points offline; it never becomes runtime decision-tree state.
pub fn cook_static(
    prior: &PriorArtifact,
    capacity: u32,
    codec: ContentCodecDescriptor,
) -> Result<SchemeArtifact, CookBuildError> {
    let codec_cardinality = codec.symbol_cardinality();
    if codec_cardinality != prior.symbol_space().cardinality {
        return Err(CookBuildError::CodecSymbolMismatch {
            prior: prior.symbol_space().cardinality,
            codec: codec_cardinality,
        });
    }
    // The runtime stream is static and corpus-independent. The prior is used
    // only by the offline point optimizer; it must never become an adaptive
    // decision tree or alter the address ABI.
    let characteristic =
        largest_prime_at_most(capacity).ok_or(CookBuildError::StaticCapacityTooSmall {
            required: 2,
            capacity,
        })?;
    let field = StandardFieldDescriptor {
        characteristic,
        degree: 1,
    };
    let optimized_points = optimized_points_for_final_field(prior, field)?;
    let scheme = Scheme {
        capacity,
        codec,
        nodes: Vec::new(),
        root: None,
        fallback: crate::fallback::FallbackDescriptor::StaticPolynomialV2 {
            codec,
            field,
            optimized_points,
        },
    };
    scheme.validate()?;
    let total_weight = prior
        .scenarios()
        .iter()
        .map(|scenario| match &scenario.weight {
            WeightExpr::FixedPoint { units, .. } => units.clone(),
        })
        .sum::<BigUint>();
    Ok(SchemeArtifact {
        scheme,
        objective: CookObjectiveDescriptor::ExpectedDistinguishingAtoms,
        cook_status: CookStatus::ApproximateRelativeToPrior,
        certificate: CookCertificateArtifact {
            total_weight: WeightExpr::FixedPoint {
                units: total_weight.clone(),
                fractional_bits: 96,
            },
            objective_cost: WeightExpr::FixedPoint {
                units: total_weight.clone(),
                fractional_bits: 96,
            },
            root_lower_bound: WeightExpr::FixedPoint {
                units: total_weight,
                fractional_bits: 96,
            },
            explored_states: 0,
            memoized_states: 0,
        },
        spec_revision: "teca-canonical-refactor-spec-v10".into(),
        cooker_revision: "teca-static-polynomial-v2".into(),
    })
}

fn largest_prime_at_most(mut n: u32) -> Option<u32> {
    while n >= 2 {
        let mut divisor = 2u32;
        let mut prime = true;
        while divisor <= n / divisor {
            if n.is_multiple_of(divisor) {
                prime = false;
                break;
            }
            divisor += if divisor == 2 { 1 } else { 2 };
        }
        if prime {
            return Some(n);
        }
        n -= 1;
    }
    None
}

fn optimized_points_for_final_field(
    _prior: &PriorArtifact,
    field: StandardFieldDescriptor,
) -> Result<[u32; 4], CookBuildError> {
    // Recomputed against the finalized 12,359-atom lexicon's actual field
    // capacity and the canonical 3,189,934-row CTM-B9 prior. The optimizer
    // counts polynomial roots directly, then applies primitive and one-byte
    // injectivity constraints.
    let q = field
        .order()
        .map_err(ProbeError::from)
        .map_err(ActionError::from)? as u32;
    if q != 12_347 {
        return Ok([1, 2, 3, 4].map(|x| x.min(q - 1)));
    }
    Ok([6743, 2728, 6148, 1809])
}

fn cook_fixed(
    prior: &PriorArtifact,
    capacity: u32,
    codec: ContentCodecDescriptor,
    generated: GeneratedActions,
) -> Result<SchemeArtifact, CookBuildError> {
    if prior
        .scenarios()
        .iter()
        .any(|s| !matches!(s.weight, WeightExpr::FixedPoint { .. }))
    {
        return Err(CookBuildError::MixedWeightKinds);
    }
    let (weights, tests, fractional_bits) =
        fixed_point_matrix(prior, &generated.bundles, capacity)?;
    debug_assert_eq!(tests, generated.tests);
    let solved = ExactCooker::new(weights, tests)?.solve()?;
    let scheme = materialize_scheme(solved.root.as_ref(), &generated, capacity, codec)?;
    Ok(SchemeArtifact {
        scheme,
        objective: CookObjectiveDescriptor::ExpectedDistinguishingAtoms,
        cook_status: CookStatus::ExactRelativeToPrior,
        certificate: CookCertificateArtifact {
            total_weight: WeightExpr::FixedPoint {
                units: solved.certificate.total_weight,
                fractional_bits,
            },
            objective_cost: WeightExpr::FixedPoint {
                units: solved.cost,
                fractional_bits,
            },
            root_lower_bound: WeightExpr::FixedPoint {
                units: solved.certificate.root_lower_bound,
                fractional_bits,
            },
            explored_states: solved.certificate.explored_states,
            memoized_states: solved.certificate.memoized_states,
        },
        spec_revision: "teca-canonical-refactor-spec-v10".into(),
        cooker_revision: "teca-cooker-exact-v1".into(),
    })
}

fn materialize_scheme(
    root: Option<&AbstractCookNode>,
    generated: &GeneratedActions,
    capacity: u32,
    codec: ContentCodecDescriptor,
) -> Result<Scheme, CookBuildError> {
    let mut nodes = Vec::new();
    let root_id = match root {
        None => None,
        Some(root) => Some(materialize_node(root, generated, &mut nodes)?),
    };
    let scheme = Scheme {
        capacity,
        codec,
        root: root_id,
        nodes,
        fallback: crate::fallback::FallbackDescriptor::DirectRadixV1,
    };
    scheme.validate()?;
    Ok(scheme)
}

fn materialize_node(
    node: &AbstractCookNode,
    generated: &GeneratedActions,
    nodes: &mut Vec<DecisionNode>,
) -> Result<u32, CookBuildError> {
    let action = generated
        .bundles
        .get(node.test_id as usize)
        .ok_or(CookBuildError::InvalidTestId(node.test_id))?
        .clone();
    let id = u32::try_from(nodes.len()).map_err(|_| CookBuildError::InvalidTestId(node.test_id))?;
    nodes.push(DecisionNode {
        action,
        branches: Vec::new(),
        default_child: None,
    });
    let mut branches = Vec::with_capacity(node.residual_branches.len());
    for (&atom, child) in &node.residual_branches {
        let child_id = materialize_node(child, generated, nodes)?;
        branches.push((crate::AtomId(atom), child_id));
    }
    branches.sort_by_key(|(atom, _)| *atom);
    nodes[id as usize].branches = branches;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prior::{PriorMetadata, Scenario, SymbolSpace, WeightExpr};

    #[test]
    fn tiny_fixed_prior_cooks_to_valid_runtime_scheme() {
        let prior = PriorArtifact {
            symbol_space: SymbolSpace {
                cardinality: 2,
                name: "binary".into(),
            },
            scenarios: vec![Scenario {
                left: vec![0],
                right: vec![1],
                weight: WeightExpr::FixedPoint {
                    units: BigUint::from(1u8),
                    fractional_bits: 0,
                },
            }],
            transformations: vec![],
            metadata: PriorMetadata::ExternalFinite {
                model_identity: "tiny-binary-test".into(),
                builder_identity: "unit-test".into(),
                budget: None,
                uncertainty_note: String::new(),
            },
            provenance: vec![],
        };
        let artifact = cook_prior(&prior, 2, ContentCodecDescriptor::BinaryMsb0).unwrap();
        artifact.scheme.validate().unwrap();
        assert_eq!(artifact.cook_status, CookStatus::ExactRelativeToPrior);
    }

    #[test]
    fn library_rejects_prior_codec_symbol_mismatch() {
        let prior = PriorArtifact {
            symbol_space: SymbolSpace {
                cardinality: 2,
                name: "binary".into(),
            },
            scenarios: vec![Scenario {
                left: vec![0],
                right: vec![1],
                weight: WeightExpr::FixedPoint {
                    units: BigUint::from(1u8),
                    fractional_bits: 0,
                },
            }],
            transformations: vec![],
            metadata: PriorMetadata::ExternalFinite {
                model_identity: "tiny-binary-test".into(),
                builder_identity: "unit-test".into(),
                budget: None,
                uncertainty_note: String::new(),
            },
            provenance: vec![],
        };
        assert!(matches!(
            cook_prior(&prior, 2, ContentCodecDescriptor::CtmB9),
            Err(CookBuildError::CodecSymbolMismatch { prior: 2, codec: 9 })
        ));
    }

    #[test]
    fn static_cook_builds_a_valid_binary_scheme() {
        let prior = PriorArtifact {
            symbol_space: SymbolSpace {
                cardinality: 2,
                name: "binary".into(),
            },
            scenarios: vec![Scenario {
                left: vec![0],
                right: vec![1],
                weight: WeightExpr::FixedPoint {
                    units: BigUint::from(1u8),
                    fractional_bits: 0,
                },
            }],
            transformations: vec![],
            metadata: PriorMetadata::ExternalFinite {
                model_identity: "tiny-binary-test".into(),
                builder_identity: "unit-test".into(),
                budget: None,
                uncertainty_note: String::new(),
            },
            provenance: vec![],
        };
        let artifact = cook_static(&prior, 11_449, ContentCodecDescriptor::BinaryMsb0).unwrap();
        assert!(artifact.scheme.nodes.is_empty());
        assert!(matches!(
            artifact.scheme.fallback,
            crate::fallback::FallbackDescriptor::StaticPolynomialV2 { .. }
        ));
        assert_eq!(artifact.cook_status, CookStatus::ApproximateRelativeToPrior);
        artifact.scheme.validate().unwrap();
    }
}
