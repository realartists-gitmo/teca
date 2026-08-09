//! Canonical offline CTM-B9-D12 prior loading and deterministic pair shaping.

use std::fs;
use std::path::Path;

use num_bigint::BigUint;

use super::artifact::{
    PriorArtifact, PriorMetadata, PriorTransform, ProvenanceRecord, Scenario, SymbolSpace,
    WeightExpr,
};

pub const CANONICAL_B9_FILENAME: &str = "ctm-b9-d12-pair-prior-fp96-v1.bin";

#[derive(Debug)]
pub enum CtmError {
    Io(std::io::Error),
    Invalid(&'static str),
}

impl std::fmt::Display for CtmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => e.fmt(f),
            Self::Invalid(s) => write!(f, "invalid canonical CTM artifact: {s}"),
        }
    }
}
impl std::error::Error for CtmError {}
impl From<std::io::Error> for CtmError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

pub fn from_fixed_point_scenarios<I, U>(
    scenarios: I,
    fractional_bits: u32,
    provenance: impl Into<String>,
) -> PriorArtifact
where
    I: IntoIterator<Item = (Vec<u32>, Vec<u32>, U)>,
    U: Into<BigUint>,
{
    let mut rows = std::collections::BTreeMap::<(Vec<u32>, Vec<u32>), BigUint>::new();
    for (mut left, mut right, units) in scenarios {
        let units = units.into();
        if units == 0u8.into() || left == right {
            continue;
        }
        if right < left {
            std::mem::swap(&mut left, &mut right);
        }
        *rows.entry((left, right)).or_default() += units;
    }
    PriorArtifact {
        symbol_space: SymbolSpace {
            cardinality: 9,
            name: "CTM-B9".into(),
        },
        scenarios: rows
            .into_iter()
            .map(|((left, right), units)| Scenario {
                left,
                right,
                weight: WeightExpr::FixedPoint {
                    units,
                    fractional_bits,
                },
            })
            .collect(),
        transformations: vec![PriorTransform::PairOrderCanonicalized],
        metadata: PriorMetadata::CanonicalCtmB9D12 {
            provenance: provenance.into(),
        },
        provenance: vec![ProvenanceRecord {
            name: "CTM-B9-D12".into(),
            source: "pybdm CTM-B9-D12 direct joint 6+6 table".into(),
            license: "MIT (pybdm source package)".into(),
        }],
    }
}

pub fn load_canonical_b9(path: impl AsRef<Path>) -> Result<PriorArtifact, CtmError> {
    let bytes = fs::read(path)?;
    let mut r = Reader { b: &bytes, p: 0 };
    if r.take(8)? != b"TECACTM1"
        || r.u32()? != 1
        || r.u32()? != 96
        || r.u32()? != 9
        || r.u32()? != 6
    {
        return Err(CtmError::Invalid("header"));
    }
    let _source_rows = r.u64()?;
    let _nonidentical_rows = r.u64()?;
    let count = r.u64()? as usize;
    // The format stores the aggregate/header audit payload between the row
    // counts and the fixed-width rows.
    r.take(72)?;
    let mut rows = Vec::with_capacity(count);
    for _ in 0..count {
        let left = digits(r.u32()?, 6);
        let right = digits(r.u32()?, 6);
        let units = BigUint::from_bytes_be(r.take(16)?);
        rows.push((left, right, units));
    }
    if r.p != bytes.len() {
        return Err(CtmError::Invalid("trailing bytes"));
    }
    Ok(from_fixed_point_scenarios(
        rows,
        96,
        "verified direct CTM-B9-D12; fixed 6+6; canonical supplied table",
    ))
}

fn digits(mut value: u32, width: usize) -> Vec<u32> {
    let mut out = vec![0; width];
    for slot in out.iter_mut().rev() {
        *slot = value % 9;
        value /= 9;
    }
    out
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CtmError> {
        let end = self.p.checked_add(n).ok_or(CtmError::Invalid("length"))?;
        if end > self.b.len() {
            return Err(CtmError::Invalid("truncated"));
        }
        let x = &self.b[self.p..end];
        self.p = end;
        Ok(x)
    }
    fn u32(&mut self) -> Result<u32, CtmError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, CtmError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
}
