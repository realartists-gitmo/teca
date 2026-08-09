//! Exact adaptive decision-tree solver over a precomputed finite test matrix.
//!
//! This module is deliberately independent of finite fields. The algebraic probe
//! generator will feed it legal candidate tests after computing each scenario's
//! left/right outcomes.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use num_bigint::BigUint;
use num_traits::Zero;

use super::certificate::ExactCertificate;
use super::tree::AbstractCookNode;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PairOutcome {
    pub left: u32,
    pub right: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateTest {
    pub id: u32,
    /// One pair outcome per scenario, in the cooker's scenario order.
    pub outcomes: Vec<PairOutcome>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactSolution {
    pub cost: BigUint,
    pub root: Option<AbstractCookNode>,
    pub certificate: ExactCertificate,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExactCookError {
    EmptyTests,
    LengthMismatch,
    UnseparableState,
}

impl fmt::Display for ExactCookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTests => write!(f, "exact cooker needs at least one candidate test"),
            Self::LengthMismatch => write!(f, "candidate-test/scenario length mismatch"),
            Self::UnseparableState => {
                write!(f, "candidate tests cannot distinguish every scenario")
            }
        }
    }
}

impl std::error::Error for ExactCookError {}

pub struct ExactCooker {
    weights: Vec<BigUint>,
    tests: Vec<CandidateTest>,
}

#[derive(Clone)]
struct MemoValue {
    cost: BigUint,
    node: Option<AbstractCookNode>,
}

struct SearchStats {
    explored: u64,
    memo_hits: u64,
}

impl ExactCooker {
    pub fn new(weights: Vec<BigUint>, tests: Vec<CandidateTest>) -> Result<Self, ExactCookError> {
        if tests.is_empty() {
            return Err(ExactCookError::EmptyTests);
        }
        if tests.iter().any(|t| t.outcomes.len() != weights.len()) {
            return Err(ExactCookError::LengthMismatch);
        }
        Ok(Self { weights, tests })
    }

    pub fn solve(&self) -> Result<ExactSolution, ExactCookError> {
        let initial: Vec<usize> = (0..self.weights.len())
            .filter(|&i| !self.weights[i].is_zero())
            .collect();
        if initial.is_empty() {
            return Ok(ExactSolution {
                cost: BigUint::zero(),
                root: None,
                certificate: ExactCertificate {
                    total_weight: BigUint::zero(),
                    optimum_cost: BigUint::zero(),
                    root_lower_bound: BigUint::zero(),
                    explored_states: 0,
                    memoized_states: 0,
                },
            });
        }
        let total = self.state_weight(&initial)?;
        let min_residual = self.min_residual_weight(&initial)?;
        let lower_bound = &total + &min_residual;
        let mut memo = HashMap::<Vec<usize>, MemoValue>::new();
        let mut stats = SearchStats {
            explored: 0,
            memo_hits: 0,
        };
        let result = self.solve_state(&initial, &mut memo, &mut stats)?;
        Ok(ExactSolution {
            cost: result.cost.clone(),
            root: result.node,
            certificate: ExactCertificate {
                total_weight: total,
                optimum_cost: result.cost,
                root_lower_bound: lower_bound,
                explored_states: stats.explored,
                memoized_states: memo.len() as u64,
            },
        })
    }

    fn solve_state(
        &self,
        state: &[usize],
        memo: &mut HashMap<Vec<usize>, MemoValue>,
        stats: &mut SearchStats,
    ) -> Result<MemoValue, ExactCookError> {
        if state.is_empty() {
            return Ok(MemoValue {
                cost: BigUint::zero(),
                node: None,
            });
        }
        if let Some(hit) = memo.get(state) {
            stats.memo_hits += 1;
            return Ok(hit.clone());
        }
        stats.explored += 1;
        let node_cost = self.state_weight(state)?;
        let mut best: Option<MemoValue> = None;

        for test in &self.tests {
            let mut branches: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
            let mut residual_count = 0usize;
            for &scenario in state {
                let outcome = test.outcomes[scenario];
                if outcome.left == outcome.right {
                    residual_count += 1;
                    branches.entry(outcome.left).or_default().push(scenario);
                }
            }
            // A test that leaves the entire state in one unchanged branch cannot
            // participate in a finite optimal tree.
            if residual_count == state.len() && branches.len() == 1 {
                continue;
            }

            let mut total = node_cost.clone();
            let mut child_nodes = BTreeMap::new();
            for (value, child_state) in branches {
                let child = self.solve_state(&child_state, memo, stats)?;
                total += &child.cost;
                if let Some(node) = child.node {
                    child_nodes.insert(value, Box::new(node));
                }
            }
            let candidate = MemoValue {
                cost: total,
                node: Some(AbstractCookNode {
                    test_id: test.id,
                    residual_branches: child_nodes,
                }),
            };
            if best.as_ref().is_none_or(|b| candidate.cost < b.cost) {
                best = Some(candidate);
            }
        }

        let best = best.ok_or(ExactCookError::UnseparableState)?;
        memo.insert(state.to_vec(), best.clone());
        Ok(best)
    }

    fn state_weight(&self, state: &[usize]) -> Result<BigUint, ExactCookError> {
        let mut out = BigUint::zero();
        for &scenario in state {
            out += &self.weights[scenario];
        }
        Ok(out)
    }

    fn min_residual_weight(&self, state: &[usize]) -> Result<BigUint, ExactCookError> {
        let mut best: Option<BigUint> = None;
        for test in &self.tests {
            let mut residual = BigUint::zero();
            for &scenario in state {
                let outcome = test.outcomes[scenario];
                if outcome.left == outcome.right {
                    residual += &self.weights[scenario];
                }
            }
            best = Some(match best {
                Some(b) => b.min(residual),
                None => residual,
            });
        }
        best.ok_or(ExactCookError::EmptyTests)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deliberately simple reference recurrence with no memoization, certificate
    /// logic, or production pruning. Used only as an independent tiny-instance
    /// oracle for the exact adaptive objective.
    fn brute_cost(
        weights: &[BigUint],
        tests: &[CandidateTest],
        state: &[usize],
    ) -> Option<BigUint> {
        if state.is_empty() {
            return Some(BigUint::zero());
        }
        let node_cost: BigUint = state.iter().map(|&i| &weights[i]).sum();
        let mut best: Option<BigUint> = None;
        for test in tests {
            let mut branches = BTreeMap::<u32, Vec<usize>>::new();
            for &scenario in state {
                let outcome = test.outcomes[scenario];
                if outcome.left == outcome.right {
                    branches.entry(outcome.left).or_default().push(scenario);
                }
            }
            if branches.len() == 1
                && branches
                    .values()
                    .next()
                    .is_some_and(|b| b.len() == state.len())
            {
                continue;
            }
            let mut total = node_cost.clone();
            let mut finite = true;
            for child in branches.values() {
                let Some(cost) = brute_cost(weights, tests, child) else {
                    finite = false;
                    break;
                };
                total += cost;
            }
            if finite && best.as_ref().is_none_or(|b| total < *b) {
                best = Some(total);
            }
        }
        best
    }

    fn ternary_outcome(code: usize) -> PairOutcome {
        match code {
            0 => PairOutcome { left: 0, right: 1 }, // resolved
            1 => PairOutcome { left: 0, right: 0 }, // residual branch 0
            2 => PairOutcome { left: 1, right: 1 }, // residual branch 1
            _ => unreachable!(),
        }
    }

    fn exhaustive_tiny_oracle(scenarios: usize, test_count: usize) {
        let weights: Vec<BigUint> = (1..=scenarios).map(BigUint::from).collect();
        let cells = scenarios * test_count;
        let total_matrices = 3usize.pow(cells as u32);
        for encoded in 0..total_matrices {
            let mut value = encoded;
            let mut tests = Vec::with_capacity(test_count);
            for test_id in 0..test_count {
                let mut outcomes = Vec::with_capacity(scenarios);
                for _ in 0..scenarios {
                    outcomes.push(ternary_outcome(value % 3));
                    value /= 3;
                }
                tests.push(CandidateTest {
                    id: test_id as u32,
                    outcomes,
                });
            }
            let state: Vec<usize> = (0..scenarios).collect();
            let expected = brute_cost(&weights, &tests, &state);
            let actual = ExactCooker::new(weights.clone(), tests).unwrap().solve();
            match (expected, actual) {
                (Some(expected), Ok(actual)) => {
                    assert_eq!(actual.cost, expected, "matrix {encoded}")
                }
                (None, Err(ExactCookError::UnseparableState)) => {}
                (expected, actual) => panic!(
                    "tiny oracle mismatch for matrix {encoded}: expected {expected:?}, actual {actual:?}"
                ),
            }
        }
    }

    #[test]
    fn exact_solver_matches_exhaustive_two_scenario_oracle() {
        exhaustive_tiny_oracle(2, 2);
    }

    #[test]
    fn exact_solver_matches_exhaustive_three_scenario_oracle() {
        exhaustive_tiny_oracle(3, 2);
    }

    #[test]
    fn adaptive_branching_beats_bad_global_second_test() {
        // Scenario 0 is resolved by test 0. Scenarios 1 and 2 collide under test
        // 0 but into different shared values, allowing different next tests.
        let weights = vec![10u32.into(), 5u32.into(), 5u32.into()];
        let tests = vec![
            CandidateTest {
                id: 0,
                outcomes: vec![
                    PairOutcome { left: 0, right: 1 },
                    PairOutcome { left: 2, right: 2 },
                    PairOutcome { left: 3, right: 3 },
                ],
            },
            CandidateTest {
                id: 1,
                outcomes: vec![
                    PairOutcome { left: 0, right: 0 },
                    PairOutcome { left: 0, right: 1 },
                    PairOutcome { left: 4, right: 4 },
                ],
            },
            CandidateTest {
                id: 2,
                outcomes: vec![
                    PairOutcome { left: 0, right: 0 },
                    PairOutcome { left: 4, right: 4 },
                    PairOutcome { left: 0, right: 1 },
                ],
            },
        ];
        let solution = ExactCooker::new(weights, tests).unwrap().solve().unwrap();
        assert_eq!(solution.cost, BigUint::from(30u32)); // 20 at root + 5 + 5 in two branches.
        assert_eq!(solution.certificate.root_lower_bound, BigUint::from(30u32));
    }
}
