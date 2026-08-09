//! Deterministic finite scalar-probe and capacity-bounded bundle generation.
//!
//! For a finite prior there are finitely many relevant Hasse orders. Scalar
//! probes and bundles are deduplicated by the exact behavior visible to the
//! adaptive objective: which scenarios are separated and the partition of the
//! scenarios that remain collided. Raw output labels that induce the same
//! partition are irrelevant. Bundle enumeration is exhaustive under the
//! lexicon-capacity product bound.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

#[cfg(test)]
use num_bigint::BigUint;

use crate::bundle::{BundleError, ProbeBundle};
use crate::codec::monic_symbol_coefficients;
use crate::field::orders::prime_powers_up_to;
use crate::field::polynomial::evaluate_hasse;
use crate::field::{ExplicitField, FieldElement, StandardFieldDescriptor};
use crate::prior::PairPrior;
use crate::probe::{FieldDescriptor, ProbeError, ScalarProbe};

use super::search::{CandidateTest, PairOutcome};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionError {
    CapacityTooSmall(u32),
    SymbolCardinalityTooSmall(u32),
    Probe(ProbeError),
    Bundle(BundleError),
    DegreeOverflow,
    TestIdOverflow,
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityTooSmall(n) => write!(f, "TECA lexicon capacity must be >=2, got {n}"),
            Self::SymbolCardinalityTooSmall(n) => {
                write!(f, "prior symbol cardinality must be >=2, got {n}")
            }
            Self::Probe(err) => err.fmt(f),
            Self::Bundle(err) => err.fmt(f),
            Self::DegreeOverflow => write!(f, "finite prior polynomial degree overflow"),
            Self::TestIdOverflow => write!(f, "generated more than u32::MAX candidate actions"),
        }
    }
}
impl std::error::Error for ActionError {}
impl From<ProbeError> for ActionError {
    fn from(v: ProbeError) -> Self {
        Self::Probe(v)
    }
}
impl From<BundleError> for ActionError {
    fn from(v: BundleError) -> Self {
        Self::Bundle(v)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GeneratedActions {
    pub bundles: Vec<ProbeBundle>,
    pub tests: Vec<CandidateTest>,
}

#[derive(Clone)]
struct ScalarCandidate {
    probe: ScalarProbe,
    cardinality: u32,
    outcomes: Vec<PairOutcome>,
}

/// A field-characteristic-specific projection of the prior.
///
/// The polynomial coefficients depend on the source symbols and the prime
/// characteristic, but not on the extension degree, point, or Hasse order.
/// Keeping one coefficient vector per distinct source string avoids rebuilding
/// the same polynomial for every probe and also avoids evaluating both sides of
/// every prior row independently.
struct ScenarioProjection {
    coefficients: Vec<Vec<u32>>,
    pairs: Vec<(usize, usize)>,
}

pub fn generate_actions(
    prior: &impl PairPrior,
    capacity: u32,
) -> Result<GeneratedActions, ActionError> {
    let scalars = generate_scalar_candidates(prior, capacity)?;
    let mut by_behavior: BTreeMap<Vec<u32>, ActionRepresentative> = BTreeMap::new();
    let mut selected = Vec::new();
    let mut memo: BTreeMap<(usize, Vec<u32>), u128> = BTreeMap::new();
    enumerate_bundles(
        &scalars,
        capacity as u128,
        0,
        1,
        &mut selected,
        None,
        &mut by_behavior,
        &mut memo,
    )?;

    // One one-atom action is never useful if another feasible one-atom action
    // strictly refines it on every prior scenario. Remove those dominated rows
    // before the adaptive dynamic program.
    let mut rows: Vec<_> = by_behavior.into_iter().collect();
    let keep: Vec<bool> = (0..rows.len())
        .map(|i| !(0..rows.len()).any(|j| i != j && behavior_refines(&rows[j].0, &rows[i].0)))
        .collect();
    rows = rows
        .into_iter()
        .zip(keep)
        .filter_map(|(row, keep)| keep.then_some(row))
        .collect();
    rows.sort_by(|(a_key, a), (b_key, b)| {
        bundle_key(&a.bundle)
            .cmp(&bundle_key(&b.bundle))
            .then_with(|| a_key.cmp(b_key))
    });

    let mut bundles = Vec::with_capacity(rows.len());
    let mut tests = Vec::with_capacity(rows.len());
    for (id, (_, representative)) in rows.into_iter().enumerate() {
        let id = u32::try_from(id).map_err(|_| ActionError::TestIdOverflow)?;
        bundles.push(representative.bundle);
        tests.push(CandidateTest {
            id,
            outcomes: representative.outcomes,
        });
    }
    Ok(GeneratedActions { bundles, tests })
}

#[derive(Clone)]
struct ActionRepresentative {
    bundle: ProbeBundle,
    outcomes: Vec<PairOutcome>,
}

fn generate_scalar_candidates(
    prior: &impl PairPrior,
    capacity: u32,
) -> Result<Vec<ScalarCandidate>, ActionError> {
    if capacity < 2 {
        return Err(ActionError::CapacityTooSmall(capacity));
    }
    let m = prior.symbol_space().cardinality;
    if m < 2 {
        return Err(ActionError::SymbolCardinalityTooSmall(m));
    }

    let mut dedup: BTreeMap<Vec<u32>, ScalarCandidate> = BTreeMap::new();
    let mut fields_by_characteristic = BTreeMap::<u32, Vec<_>>::new();
    for field in prime_powers_up_to(capacity) {
        fields_by_characteristic
            .entry(field.characteristic)
            .or_default()
            .push(field);
    }
    for (characteristic, fields) in fields_by_characteristic {
        let projection = compile_projection(prior, m, characteristic)?;
        let max_degree = projection
            .coefficients
            .iter()
            .map(|coefficients| coefficients.len().saturating_sub(1))
            .max()
            .unwrap_or(0);
        for field in fields {
            let desc = StandardFieldDescriptor {
                characteristic,
                degree: field.degree,
            };
            // Instantiate once per field. The compiled projection is shared by
            // every point and Hasse order in this field.
            let explicit = desc.instantiate().map_err(ProbeError::from)?;
            for point_rank in 0..field.order as u128 {
                let point = explicit.from_rank(point_rank).map_err(ProbeError::from)?;
                for hasse_order in 0..=max_degree {
                    let mut values = Vec::with_capacity(projection.coefficients.len());
                    for coefficients in &projection.coefficients {
                        let value =
                            evaluate_compiled(&explicit, coefficients, hasse_order, &point)?;
                        values.push(value);
                    }
                    let mut outcomes = Vec::with_capacity(projection.pairs.len());
                    let mut distinguishes = false;
                    for &(left_index, right_index) in &projection.pairs {
                        let row = PairOutcome {
                            left: values[left_index],
                            right: values[right_index],
                        };
                        distinguishes |= row.left != row.right;
                        outcomes.push(row);
                    }
                    let probe = ScalarProbe {
                        field: FieldDescriptor::Standard(desc),
                        point_rank,
                        hasse_order: u32::try_from(hasse_order)
                            .map_err(|_| ActionError::DegreeOverflow)?,
                    };
                    if !distinguishes {
                        continue;
                    }
                    let behavior = behavior_key(&outcomes);
                    let candidate = ScalarCandidate {
                        probe,
                        cardinality: field.order,
                        outcomes,
                    };
                    match dedup.get(&behavior) {
                        None => {
                            dedup.insert(behavior, candidate);
                        }
                        Some(old)
                            if (candidate.cardinality, scalar_key(&candidate.probe))
                                < (old.cardinality, scalar_key(&old.probe)) =>
                        {
                            dedup.insert(behavior, candidate);
                        }
                        Some(_) => {}
                    }
                }
            }
        }
    }
    let mut out: Vec<_> = dedup.into_values().collect();
    out.sort_by_key(|c| (c.cardinality, scalar_key(&c.probe)));

    // Scalar A dominates scalar B if A costs no more radix capacity and its
    // resolved/residual partition refines B. Any bundle containing B can replace
    // B with A without losing information or exceeding capacity.
    let behaviors: Vec<_> = out.iter().map(|c| behavior_key(&c.outcomes)).collect();
    let keep: Vec<bool> = (0..out.len())
        .map(|i| {
            !(0..out.len()).any(|j| {
                i != j
                    && out[j].cardinality <= out[i].cardinality
                    && behavior_refines(&behaviors[j], &behaviors[i])
            })
        })
        .collect();
    out = out
        .into_iter()
        .zip(keep)
        .filter_map(|(candidate, keep)| keep.then_some(candidate))
        .collect();
    out.sort_by_key(|c| scalar_key(&c.probe));
    Ok(out)
}

fn compile_projection(
    prior: &impl PairPrior,
    symbol_cardinality: u32,
    characteristic: u32,
) -> Result<ScenarioProjection, ActionError> {
    let mut sequence_ids = HashMap::<Vec<u32>, usize>::new();
    let mut sequences = Vec::<Vec<u32>>::new();
    let mut pairs = Vec::with_capacity(prior.scenarios().len());
    for scenario in prior.scenarios() {
        let left = intern_sequence(&mut sequence_ids, &mut sequences, &scenario.left);
        let right = intern_sequence(&mut sequence_ids, &mut sequences, &scenario.right);
        pairs.push((left, right));
    }
    let coefficients = sequences
        .iter()
        .map(|symbols| {
            monic_symbol_coefficients(symbols, symbol_cardinality, characteristic)
                .map_err(ProbeError::from)
                .map_err(ActionError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ScenarioProjection {
        coefficients,
        pairs,
    })
}

fn intern_sequence(
    ids: &mut HashMap<Vec<u32>, usize>,
    sequences: &mut Vec<Vec<u32>>,
    sequence: &[u32],
) -> usize {
    if let Some(&id) = ids.get(sequence) {
        return id;
    }
    let id = sequences.len();
    let owned = sequence.to_vec();
    ids.insert(owned.clone(), id);
    sequences.push(owned);
    id
}

fn evaluate_compiled(
    field: &ExplicitField,
    coefficients: &[u32],
    hasse_order: usize,
    point: &FieldElement,
) -> Result<u32, ActionError> {
    let value = evaluate_hasse(field, coefficients, hasse_order, point)
        .map_err(ProbeError::from)
        .map_err(ActionError::from)?;
    Ok(field
        .rank(&value)
        .map_err(ProbeError::from)
        .map_err(ActionError::from)? as u32)
}

#[allow(clippy::too_many_arguments)]
fn enumerate_bundles(
    scalars: &[ScalarCandidate],
    capacity: u128,
    start: usize,
    product: u128,
    selected: &mut Vec<usize>,
    combined: Option<Vec<PairOutcome>>,
    out: &mut BTreeMap<Vec<u32>, ActionRepresentative>,
    memo: &mut BTreeMap<(usize, Vec<u32>), u128>,
) -> Result<(), ActionError> {
    let current_behavior = combined.as_ref().map(|rows| behavior_key(rows));
    if let Some(key) = &current_behavior {
        let state = (start, key.clone());
        if memo
            .get(&state)
            .is_some_and(|&best_product| best_product <= product)
        {
            return Ok(());
        }
        memo.insert(state, product);
        if key.iter().all(|&x| x == RESOLVED) {
            return Ok(());
        }
    }

    for i in start..scalars.len() {
        let scalar = &scalars[i];
        let Some(next_product) = product.checked_mul(scalar.cardinality as u128) else {
            continue;
        };
        if next_product > capacity {
            continue;
        }
        let next_outcomes = match &combined {
            None => scalar.outcomes.clone(),
            Some(previous) => previous
                .iter()
                .zip(&scalar.outcomes)
                .map(|(a, b)| PairOutcome {
                    left: (a.left as u128 * scalar.cardinality as u128 + b.left as u128) as u32,
                    right: (a.right as u128 * scalar.cardinality as u128 + b.right as u128) as u32,
                })
                .collect(),
        };
        let behavior = behavior_key(&next_outcomes);
        if current_behavior
            .as_ref()
            .is_some_and(|old| *old == behavior)
        {
            continue; // consumes capacity without refining any prior scenario.
        }

        selected.push(i);
        let bundle =
            ProbeBundle::new(selected.iter().map(|&j| scalars[j].probe.clone()).collect())?;
        let representative = ActionRepresentative {
            bundle,
            outcomes: next_outcomes.clone(),
        };
        match out.get(&behavior) {
            None => {
                out.insert(behavior.clone(), representative);
            }
            Some(old)
                if (next_product, bundle_key(&representative.bundle))
                    < (old.bundle.outcome_cardinality()?, bundle_key(&old.bundle)) =>
            {
                out.insert(behavior.clone(), representative);
            }
            Some(_) => {}
        }
        enumerate_bundles(
            scalars,
            capacity,
            i + 1,
            next_product,
            selected,
            Some(next_outcomes),
            out,
            memo,
        )?;
        selected.pop();
    }
    Ok(())
}

const RESOLVED: u32 = u32::MAX;

/// Canonical behavioral signature of one action. Separated scenarios use the
/// distinguished marker `RESOLVED`; collided outcomes are replaced by first-use
/// partition IDs. Raw field labels cannot affect expected distinguishing depth.
fn behavior_key(outcomes: &[PairOutcome]) -> Vec<u32> {
    let mut groups = BTreeMap::<u32, u32>::new();
    let mut next = 0u32;
    outcomes
        .iter()
        .map(|row| {
            if row.left != row.right {
                RESOLVED
            } else {
                *groups.entry(row.left).or_insert_with(|| {
                    let group = next;
                    next += 1;
                    group
                })
            }
        })
        .collect()
}

/// True when behavior `a` is at least as informative as `b`: every scenario
/// resolved by `b` is resolved by `a`, and every residual block of `a` is
/// contained in one residual block of `b`.
fn behavior_refines(a: &[u32], b: &[u32]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut block_map = BTreeMap::<u32, u32>::new();
    for (&aa, &bb) in a.iter().zip(b) {
        if bb == RESOLVED {
            if aa != RESOLVED {
                return false;
            }
            continue;
        }
        if aa == RESOLVED {
            continue;
        }
        match block_map.get(&aa) {
            None => {
                block_map.insert(aa, bb);
            }
            Some(&expected) if expected == bb => {}
            Some(_) => return false,
        }
    }
    true
}

fn scalar_key(probe: &ScalarProbe) -> (u32, u32, u128, u32) {
    match &probe.field {
        FieldDescriptor::Standard(desc) => (
            desc.characteristic,
            desc.degree,
            probe.point_rank,
            probe.hasse_order,
        ),
    }
}

fn bundle_key(bundle: &ProbeBundle) -> Vec<(u32, u32, u128, u32)> {
    bundle.probes.iter().map(scalar_key).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prior::artifact::{PriorArtifact, PriorMetadata, Scenario, SymbolSpace, WeightExpr};

    #[test]
    fn behavioral_keys_ignore_irrelevant_output_labels() {
        let a = vec![
            PairOutcome { left: 7, right: 7 },
            PairOutcome { left: 9, right: 3 },
            PairOutcome { left: 7, right: 7 },
            PairOutcome { left: 4, right: 4 },
        ];
        let b = vec![
            PairOutcome { left: 2, right: 2 },
            PairOutcome { left: 8, right: 1 },
            PairOutcome { left: 2, right: 2 },
            PairOutcome { left: 6, right: 6 },
        ];
        assert_eq!(behavior_key(&a), behavior_key(&b));
    }

    #[test]
    fn refinement_is_information_monotone() {
        let coarse = vec![0, 0, 1, RESOLVED];
        let fine = vec![0, 1, 2, RESOLVED];
        assert!(behavior_refines(&fine, &coarse));
        assert!(!behavior_refines(&coarse, &fine));
    }

    #[test]
    fn tiny_action_generation_is_finite_and_capacity_bounded() {
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
        let generated = generate_actions(&prior, 2).unwrap();
        assert!(!generated.bundles.is_empty());
        assert_eq!(generated.bundles.len(), generated.tests.len());
        assert!(
            generated
                .bundles
                .iter()
                .all(|b| b.outcome_cardinality().unwrap() <= 2)
        );
    }
}
