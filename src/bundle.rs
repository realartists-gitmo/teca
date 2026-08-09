use std::fmt;

use crate::address::AtomId;
use crate::codec::ContentCodecDescriptor;
use crate::probe::{ProbeError, ScalarProbe};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeBundle {
    pub probes: Vec<ScalarProbe>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BundleError {
    Empty,
    Probe(ProbeError),
    OutcomeOverflow,
    CapacityExceeded { outcomes: u128, capacity: u32 },
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "probe bundle must contain at least one scalar probe"),
            Self::Probe(err) => err.fmt(f),
            Self::OutcomeOverflow => write!(f, "probe-bundle outcome cardinality overflowed"),
            Self::CapacityExceeded { outcomes, capacity } => write!(
                f,
                "probe bundle has {outcomes} possible outcomes but lexicon capacity is {capacity}"
            ),
        }
    }
}

impl std::error::Error for BundleError {}

impl From<ProbeError> for BundleError {
    fn from(value: ProbeError) -> Self {
        Self::Probe(value)
    }
}

impl ProbeBundle {
    pub fn new(probes: Vec<ScalarProbe>) -> Result<Self, BundleError> {
        if probes.is_empty() {
            return Err(BundleError::Empty);
        }
        Ok(Self { probes })
    }

    pub fn evaluate(
        &self,
        bytes: &[u8],
        codec: ContentCodecDescriptor,
        capacity: u32,
    ) -> Result<AtomId, BundleError> {
        let symbols = codec.symbols(bytes);
        self.evaluate_symbols(&symbols, codec.symbol_cardinality(), capacity)
    }

    pub fn evaluate_symbols(
        &self,
        symbols: &[u32],
        symbol_cardinality: u32,
        capacity: u32,
    ) -> Result<AtomId, BundleError> {
        if self.probes.is_empty() {
            return Err(BundleError::Empty);
        }
        let mut rank = 0u128;
        let mut outcomes = 1u128;
        for probe in &self.probes {
            let (value, q) = probe.evaluate_symbols(symbols, symbol_cardinality)?;
            outcomes = outcomes
                .checked_mul(q)
                .ok_or(BundleError::OutcomeOverflow)?;
            if outcomes > capacity as u128 {
                return Err(BundleError::CapacityExceeded { outcomes, capacity });
            }
            rank = rank
                .checked_mul(q)
                .and_then(|x| x.checked_add(value))
                .ok_or(BundleError::OutcomeOverflow)?;
        }
        debug_assert!(rank < outcomes);
        Ok(AtomId(rank as u32))
    }

    pub fn outcome_cardinality(&self) -> Result<u128, BundleError> {
        if self.probes.is_empty() {
            return Err(BundleError::Empty);
        }
        let mut outcomes = 1u128;
        for probe in &self.probes {
            let q = probe.field_cardinality()?;
            outcomes = outcomes
                .checked_mul(q)
                .ok_or(BundleError::OutcomeOverflow)?;
        }
        Ok(outcomes)
    }

    /// Full artifact/runtime validation, including StandardFF construction and
    /// probe-point bounds, with the resulting mixed-radix capacity.
    pub fn validate(&self, capacity: u32) -> Result<u128, BundleError> {
        if self.probes.is_empty() {
            return Err(BundleError::Empty);
        }
        let mut outcomes = 1u128;
        for probe in &self.probes {
            let q = probe.validate()?;
            outcomes = outcomes
                .checked_mul(q)
                .ok_or(BundleError::OutcomeOverflow)?;
            if outcomes > capacity as u128 {
                return Err(BundleError::CapacityExceeded { outcomes, capacity });
            }
        }
        Ok(outcomes)
    }
}
