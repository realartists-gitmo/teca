//! Canonical byte-stable TECA artifact serialization.
//!
//! All integers are big-endian, maps are serialized in explicit sorted order,
//! strings/byte strings are length-prefixed. The three artifact classes are
//! independent and explicitly versioned by magic.

use std::fmt;

use num_bigint::BigUint;
use num_traits::Zero;

use crate::address::AtomId;
use crate::bundle::ProbeBundle;
use crate::codec::ContentCodecDescriptor;
use crate::fallback::FallbackDescriptor;
use crate::field::standardff::StandardFieldDescriptor;
use crate::lexicon::{Lexicon, LexiconError};
use crate::prior::{
    PriorArtifact, PriorMetadata, PriorTransform, ProvenanceRecord, Scenario, SymbolSpace,
    WeightExpr,
};
use crate::probe::{FieldDescriptor, ScalarProbe};
use crate::scheme::{DecisionNode, Scheme, SchemeError};

const SCHEME_MAGIC_V2: &[u8; 8] = b"TECASM02";
const PRIOR_MAGIC_V2: &[u8; 8] = b"TECAPR02";
const LEXICON_MAGIC_V2: &[u8; 8] = b"TECALX02";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CookObjectiveDescriptor {
    ExpectedDistinguishingAtoms,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CookStatus {
    ExactRelativeToPrior,
    ApproximateRelativeToPrior,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CookCertificateArtifact {
    pub total_weight: WeightExpr,
    pub objective_cost: WeightExpr,
    pub root_lower_bound: WeightExpr,
    pub explored_states: u64,
    pub memoized_states: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchemeArtifact {
    pub scheme: Scheme,
    pub objective: CookObjectiveDescriptor,
    pub cook_status: CookStatus,
    pub certificate: CookCertificateArtifact,
    pub spec_revision: String,
    pub cooker_revision: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LexiconArtifact {
    pub lexicon: Lexicon,
    pub renderer: String,
    pub profile: String,
    /// Deterministic validation claims frozen with this exact atom ordering.
    pub validation: Vec<String>,
    /// Representative tokenizer/model families used to derive this profile,
    /// when applicable. This is provenance only; literal strings never enter
    /// scheme cooking.
    pub tokenizer_families: Vec<String>,
    pub provenance: Vec<ProvenanceRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArtifactError {
    Truncated,
    TrailingBytes,
    BadMagic,
    InvalidUtf8,
    UnsupportedTag { what: &'static str, tag: u8 },
    CountOverflow,
    InvalidScheme(SchemeError),
    InvalidLexicon(LexiconError),
    InvalidLexiconMetadata(&'static str),
    InvalidField,
    InvalidPrior(&'static str),
    UnsupportedFallbackForFormat(&'static str),
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "truncated TECA artifact"),
            Self::TrailingBytes => write!(f, "TECA artifact contains trailing bytes"),
            Self::BadMagic => write!(f, "invalid or unsupported TECA artifact magic"),
            Self::InvalidUtf8 => write!(f, "artifact string is not valid UTF-8"),
            Self::UnsupportedTag { what, tag } => write!(f, "unsupported {what} tag {tag}"),
            Self::CountOverflow => write!(f, "artifact count does not fit the encoded width"),
            Self::InvalidScheme(err) => err.fmt(f),
            Self::InvalidLexicon(err) => err.fmt(f),
            Self::InvalidLexiconMetadata(msg) => {
                write!(f, "invalid lexicon artifact metadata: {msg}")
            }
            Self::InvalidField => write!(f, "invalid field descriptor in artifact"),
            Self::InvalidPrior(msg) => write!(f, "invalid prior artifact: {msg}"),
            Self::UnsupportedFallbackForFormat(msg) => {
                write!(f, "unsupported fallback for artifact format: {msg}")
            }
        }
    }
}
impl std::error::Error for ArtifactError {}

pub fn encode_scheme(artifact: &SchemeArtifact) -> Result<Vec<u8>, ArtifactError> {
    artifact
        .scheme
        .validate()
        .map_err(ArtifactError::InvalidScheme)?;
    validate_certificate_kind(&artifact.certificate)?;
    let mut out = Vec::new();
    out.extend_from_slice(SCHEME_MAGIC_V2);
    out.push(match artifact.objective {
        CookObjectiveDescriptor::ExpectedDistinguishingAtoms => 0,
    });
    out.push(match artifact.cook_status {
        CookStatus::ExactRelativeToPrior => 0,
        CookStatus::ApproximateRelativeToPrior => 1,
    });
    put_string(&mut out, &artifact.spec_revision)?;
    put_string(&mut out, &artifact.cooker_revision)?;
    put_weight(&mut out, &artifact.certificate.total_weight)?;
    put_weight(&mut out, &artifact.certificate.objective_cost)?;
    put_weight(&mut out, &artifact.certificate.root_lower_bound)?;
    put_u64(&mut out, artifact.certificate.explored_states);
    put_u64(&mut out, artifact.certificate.memoized_states);
    put_scheme_payload(&mut out, &artifact.scheme)?;
    Ok(out)
}

pub fn decode_scheme(bytes: &[u8]) -> Result<SchemeArtifact, ArtifactError> {
    let payload = verified_payload(bytes, SCHEME_MAGIC_V2)?;
    let mut r = Reader {
        bytes: payload,
        pos: 8,
    };
    let objective = match r.u8()? {
        0 => CookObjectiveDescriptor::ExpectedDistinguishingAtoms,
        tag => {
            return Err(ArtifactError::UnsupportedTag {
                what: "cook objective",
                tag,
            });
        }
    };
    let cook_status = match r.u8()? {
        0 => CookStatus::ExactRelativeToPrior,
        1 => CookStatus::ApproximateRelativeToPrior,
        tag => {
            return Err(ArtifactError::UnsupportedTag {
                what: "cook status",
                tag,
            });
        }
    };
    let spec_revision = r.string()?;
    let cooker_revision = r.string()?;
    let certificate = CookCertificateArtifact {
        total_weight: r.weight()?,
        objective_cost: r.weight()?,
        root_lower_bound: r.weight()?,
        explored_states: r.u64()?,
        memoized_states: r.u64()?,
    };
    validate_certificate_kind(&certificate)?;
    let scheme = r.scheme_payload()?;
    r.finish()?;
    Ok(SchemeArtifact {
        scheme,
        objective,
        cook_status,
        certificate,
        spec_revision,
        cooker_revision,
    })
}

pub fn encode_prior(prior: &PriorArtifact) -> Result<Vec<u8>, ArtifactError> {
    validate_prior(prior)?;
    let mut out = Vec::new();
    out.extend_from_slice(PRIOR_MAGIC_V2);
    put_u32(&mut out, prior.symbol_space.cardinality);
    put_string(&mut out, &prior.symbol_space.name)?;

    let mut scenarios = prior.scenarios.clone();
    scenarios.sort_by(|a, b| {
        a.left
            .cmp(&b.left)
            .then_with(|| a.right.cmp(&b.right))
            .then_with(|| compare_weights_structural(&a.weight, &b.weight))
    });
    put_count(&mut out, scenarios.len())?;
    for scenario in &scenarios {
        put_symbols(&mut out, &scenario.left)?;
        put_symbols(&mut out, &scenario.right)?;
        put_weight(&mut out, &scenario.weight)?;
    }
    put_transformations(&mut out, &prior.transformations)?;
    put_prior_metadata(&mut out, &prior.metadata)?;
    put_provenance(&mut out, &prior.provenance)?;
    Ok(out)
}

pub fn decode_prior(bytes: &[u8]) -> Result<PriorArtifact, ArtifactError> {
    let payload = verified_payload(bytes, PRIOR_MAGIC_V2)?;
    let mut r = Reader {
        bytes: payload,
        pos: 8,
    };
    let symbol_space = SymbolSpace {
        cardinality: r.u32()?,
        name: r.string()?,
    };
    let count = r.count()?;
    let mut scenarios = Vec::with_capacity(count);
    for _ in 0..count {
        scenarios.push(Scenario {
            left: r.symbols()?,
            right: r.symbols()?,
            weight: r.weight()?,
        });
    }
    if scenarios
        .windows(2)
        .any(|w| (&w[0].left, &w[0].right) >= (&w[1].left, &w[1].right))
    {
        return Err(ArtifactError::InvalidPrior(
            "serialized prior scenarios are not in canonical pair order",
        ));
    }
    let transformations = r.transformations()?;
    let metadata = r.prior_metadata()?;
    let provenance = r.provenance()?;
    r.finish()?;
    let prior = PriorArtifact {
        symbol_space,
        scenarios,
        transformations,
        metadata,
        provenance,
    };
    validate_prior(&prior)?;
    Ok(prior)
}

pub fn encode_lexicon(artifact: &LexiconArtifact) -> Result<Vec<u8>, ArtifactError> {
    validate_lexicon_artifact(artifact)?;
    let mut out = Vec::new();
    out.extend_from_slice(LEXICON_MAGIC_V2);
    put_count(&mut out, artifact.lexicon.capacity())?;
    for atom in artifact.lexicon.atoms() {
        put_bytes(&mut out, atom)?;
    }
    put_string(&mut out, &artifact.renderer)?;
    put_string(&mut out, &artifact.profile)?;
    put_strings(&mut out, &artifact.validation)?;
    put_strings(&mut out, &artifact.tokenizer_families)?;
    put_provenance(&mut out, &artifact.provenance)?;
    Ok(out)
}

pub fn decode_lexicon(bytes: &[u8]) -> Result<LexiconArtifact, ArtifactError> {
    let payload = verified_payload(bytes, LEXICON_MAGIC_V2)?;
    let mut r = Reader {
        bytes: payload,
        pos: 8,
    };
    let count = r.count()?;
    let mut atoms = Vec::with_capacity(count);
    for _ in 0..count {
        atoms.push(r.bytes_vec()?);
    }
    let lexicon = Lexicon::new(atoms).map_err(ArtifactError::InvalidLexicon)?;
    let renderer = r.string()?;
    let profile = r.string()?;
    let validation = r.strings()?;
    let tokenizer_families = r.strings()?;
    let provenance = r.provenance()?;
    r.finish()?;
    let artifact = LexiconArtifact {
        lexicon,
        renderer,
        profile,
        validation,
        tokenizer_families,
        provenance,
    };
    validate_lexicon_artifact(&artifact)?;
    Ok(artifact)
}

fn validate_prior(prior: &PriorArtifact) -> Result<(), ArtifactError> {
    if prior.symbol_space.cardinality < 2 {
        return Err(ArtifactError::InvalidPrior(
            "symbol cardinality must be >=2",
        ));
    }
    let mut pairs = std::collections::BTreeSet::new();
    if prior.transformations.windows(2).any(|w| w[0] >= w[1]) {
        return Err(ArtifactError::InvalidPrior(
            "prior transformations must be strictly sorted and unique",
        ));
    }
    let pair_canonicalized = prior
        .transformations
        .contains(&PriorTransform::PairOrderCanonicalized);
    if prior
        .transformations
        .contains(&PriorTransform::GlobalBinaryComplementAggregated)
        && prior.symbol_space.cardinality != 2
    {
        return Err(ArtifactError::InvalidPrior(
            "global binary-complement transform requires a binary symbol space",
        ));
    }
    let mut fixed_scale = None;
    let mut previous_pair: Option<(&[u32], &[u32])> = None;
    for s in &prior.scenarios {
        if previous_pair.is_some_and(|previous| previous >= (s.left.as_slice(), s.right.as_slice()))
        {
            return Err(ArtifactError::InvalidPrior(
                "prior scenarios must be in strict canonical pair order",
            ));
        }
        previous_pair = Some((&s.left, &s.right));
        if s.left == s.right {
            return Err(ArtifactError::InvalidPrior(
                "identical pair scenarios carry no distinction information",
            ));
        }
        if pair_canonicalized && s.right < s.left {
            return Err(ArtifactError::InvalidPrior(
                "pair-order-canonicalized prior contains reversed scenario",
            ));
        }
        if !pairs.insert((s.left.clone(), s.right.clone())) {
            return Err(ArtifactError::InvalidPrior(
                "duplicate pair scenario must be aggregated",
            ));
        }
        if s.left
            .iter()
            .chain(&s.right)
            .any(|&v| v >= prior.symbol_space.cardinality)
        {
            return Err(ArtifactError::InvalidPrior(
                "scenario symbol outside symbol space",
            ));
        }
        match &s.weight {
            WeightExpr::FixedPoint {
                units,
                fractional_bits,
            } => {
                if units.is_zero() {
                    return Err(ArtifactError::InvalidPrior(
                        "zero-weight scenario must be omitted",
                    ));
                }
                match fixed_scale {
                    None => fixed_scale = Some(*fractional_bits),
                    Some(scale) if scale != *fractional_bits => {
                        return Err(ArtifactError::InvalidPrior(
                            "fixed-point scenario scales differ",
                        ));
                    }
                    Some(_) => {}
                }
            }
        }
    }

    match &prior.metadata {
        PriorMetadata::CanonicalCtmB9D12 { .. } => {
            if prior.symbol_space.cardinality != 9 {
                return Err(ArtifactError::InvalidPrior(
                    "canonical CTM-B9 metadata requires cardinality 9",
                ));
            }
            if prior
                .scenarios
                .iter()
                .any(|s| !matches!(s.weight, WeightExpr::FixedPoint { .. }))
            {
                return Err(ArtifactError::InvalidPrior(
                    "canonical CTM metadata requires fixed-point weights",
                ));
            }
        }
        PriorMetadata::ExternalFinite {
            model_identity,
            builder_identity,
            ..
        } => {
            if model_identity.is_empty() || builder_identity.is_empty() {
                return Err(ArtifactError::InvalidPrior(
                    "external finite prior requires nonempty model and builder identities",
                ));
            }
        }
    }
    Ok(())
}

fn validate_lexicon_artifact(artifact: &LexiconArtifact) -> Result<(), ArtifactError> {
    if artifact.renderer.is_empty() || artifact.profile.is_empty() {
        return Err(ArtifactError::InvalidLexiconMetadata(
            "renderer/profile cannot be empty",
        ));
    }
    if artifact.validation.is_empty() || artifact.validation.iter().any(String::is_empty) {
        return Err(ArtifactError::InvalidLexiconMetadata(
            "at least one nonempty validation claim is required",
        ));
    }
    let mut families = std::collections::BTreeSet::new();
    if artifact
        .tokenizer_families
        .iter()
        .any(|name| name.is_empty() || !families.insert(name))
    {
        return Err(ArtifactError::InvalidLexiconMetadata(
            "tokenizer-family provenance contains an empty or duplicate entry",
        ));
    }
    Ok(())
}

fn validate_certificate_kind(cert: &CookCertificateArtifact) -> Result<(), ArtifactError> {
    match (
        &cert.total_weight,
        &cert.objective_cost,
        &cert.root_lower_bound,
    ) {
        (
            WeightExpr::FixedPoint {
                units: total,
                fractional_bits: a,
            },
            WeightExpr::FixedPoint {
                units: objective,
                fractional_bits: b,
            },
            WeightExpr::FixedPoint {
                units: lower,
                fractional_bits: c,
            },
        ) if a == b && a == c => {
            if total <= lower && lower <= objective {
                Ok(())
            } else {
                Err(ArtifactError::InvalidPrior(
                    "certificate must satisfy total_weight <= root_lower_bound <= objective_cost",
                ))
            }
        }
        (
            WeightExpr::FixedPoint { .. },
            WeightExpr::FixedPoint { .. },
            WeightExpr::FixedPoint { .. },
        ) => Err(ArtifactError::InvalidPrior(
            "certificate fixed-point scales differ",
        )),
    }
}

fn compare_weights_structural(a: &WeightExpr, b: &WeightExpr) -> std::cmp::Ordering {
    match (a, b) {
        (
            WeightExpr::FixedPoint {
                units: au,
                fractional_bits: af,
            },
            WeightExpr::FixedPoint {
                units: bu,
                fractional_bits: bf,
            },
        ) => af.cmp(bf).then_with(|| au.cmp(bu)),
    }
}

fn put_scheme_payload(out: &mut Vec<u8>, scheme: &Scheme) -> Result<(), ArtifactError> {
    put_u32(out, scheme.capacity);
    out.push(match scheme.codec {
        ContentCodecDescriptor::BinaryMsb0 => 0,
        ContentCodecDescriptor::CtmB9 => 1,
    });
    out.push(match scheme.fallback {
        FallbackDescriptor::DirectRadixV1 => 1,
        FallbackDescriptor::StaticPolynomialV2 { .. } => 2,
    });
    if let FallbackDescriptor::StaticPolynomialV2 {
        codec,
        field,
        optimized_points,
    } = scheme.fallback
    {
        out.push(match codec {
            ContentCodecDescriptor::BinaryMsb0 => 0,
            ContentCodecDescriptor::CtmB9 => 1,
        });
        put_u32(out, field.characteristic);
        put_u32(out, field.degree);
        for point in optimized_points {
            put_u32(out, point);
        }
    }
    put_optional_u32(out, scheme.root);
    put_count(out, scheme.nodes.len())?;
    for node in &scheme.nodes {
        put_count(out, node.action.probes.len())?;
        for probe in &node.action.probes {
            put_probe(out, probe);
        }
        put_count(out, node.branches.len())?;
        for &(atom, child) in &node.branches {
            put_u32(out, atom.0);
            put_u32(out, child);
        }
        put_optional_u32(out, node.default_child);
    }
    Ok(())
}

fn put_probe(out: &mut Vec<u8>, probe: &ScalarProbe) {
    match &probe.field {
        FieldDescriptor::Standard(d) => {
            out.push(0);
            put_u32(out, d.characteristic);
            put_u32(out, d.degree);
        }
    }
    put_u128(out, probe.point_rank);
    put_u32(out, probe.hasse_order);
}

fn put_weight(out: &mut Vec<u8>, weight: &WeightExpr) -> Result<(), ArtifactError> {
    match weight {
        WeightExpr::FixedPoint {
            units,
            fractional_bits,
        } => {
            out.push(0);
            put_biguint(out, units)?;
            put_u32(out, *fractional_bits);
        }
    }
    Ok(())
}

fn put_prior_metadata(out: &mut Vec<u8>, metadata: &PriorMetadata) -> Result<(), ArtifactError> {
    match metadata {
        PriorMetadata::CanonicalCtmB9D12 { provenance } => {
            out.push(0);
            put_string(out, provenance)?;
        }
        PriorMetadata::ExternalFinite {
            model_identity,
            builder_identity,
            budget,
            uncertainty_note,
        } => {
            out.push(2);
            put_string(out, model_identity)?;
            put_string(out, builder_identity)?;
            match budget {
                None => out.push(0),
                Some(value) => {
                    out.push(1);
                    put_string(out, value)?;
                }
            }
            put_string(out, uncertainty_note)?;
        }
    }
    Ok(())
}

fn put_transformations(out: &mut Vec<u8>, rows: &[PriorTransform]) -> Result<(), ArtifactError> {
    put_count(out, rows.len())?;
    for row in rows {
        match row {
            PriorTransform::PairOrderCanonicalized => out.push(0),
            PriorTransform::GlobalBinaryComplementAggregated => out.push(1),
            PriorTransform::Named(name) => {
                out.push(2);
                put_string(out, name)?;
            }
        }
    }
    Ok(())
}

fn put_provenance(out: &mut Vec<u8>, rows: &[ProvenanceRecord]) -> Result<(), ArtifactError> {
    put_count(out, rows.len())?;
    for row in rows {
        put_string(out, &row.name)?;
        put_string(out, &row.source)?;
        put_string(out, &row.license)?;
    }
    Ok(())
}

fn put_strings(out: &mut Vec<u8>, rows: &[String]) -> Result<(), ArtifactError> {
    put_count(out, rows.len())?;
    for row in rows {
        put_string(out, row)?;
    }
    Ok(())
}

fn put_symbols(out: &mut Vec<u8>, symbols: &[u32]) -> Result<(), ArtifactError> {
    put_count(out, symbols.len())?;
    for &s in symbols {
        put_u32(out, s);
    }
    Ok(())
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), ArtifactError> {
    let len = u32::try_from(bytes.len()).map_err(|_| ArtifactError::CountOverflow)?;
    put_u32(out, len);
    out.extend_from_slice(bytes);
    Ok(())
}
fn put_string(out: &mut Vec<u8>, value: &str) -> Result<(), ArtifactError> {
    put_bytes(out, value.as_bytes())
}
fn put_count(out: &mut Vec<u8>, value: usize) -> Result<(), ArtifactError> {
    put_u32(
        out,
        u32::try_from(value).map_err(|_| ArtifactError::CountOverflow)?,
    );
    Ok(())
}
fn put_optional_u32(out: &mut Vec<u8>, value: Option<u32>) {
    match value {
        None => out.push(0),
        Some(v) => {
            out.push(1);
            put_u32(out, v);
        }
    }
}
fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn put_u128(out: &mut Vec<u8>, value: u128) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn put_biguint(out: &mut Vec<u8>, value: &BigUint) -> Result<(), ArtifactError> {
    if value.is_zero() {
        return put_bytes(out, &[]);
    }
    let bytes = value.to_bytes_be();
    put_bytes(out, &bytes)
}

fn verified_payload<'a>(bytes: &'a [u8], magic: &[u8; 8]) -> Result<&'a [u8], ArtifactError> {
    if bytes.len() < 8 {
        return Err(ArtifactError::Truncated);
    }
    if bytes.get(..8) != Some(magic.as_slice()) {
        return Err(ArtifactError::BadMagic);
    }
    Ok(bytes)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn finish(&self) -> Result<(), ArtifactError> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(ArtifactError::TrailingBytes)
        }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], ArtifactError> {
        let end = self.pos.checked_add(n).ok_or(ArtifactError::Truncated)?;
        let out = self
            .bytes
            .get(self.pos..end)
            .ok_or(ArtifactError::Truncated)?;
        self.pos = end;
        Ok(out)
    }
    fn u8(&mut self) -> Result<u8, ArtifactError> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, ArtifactError> {
        let mut b = [0; 4];
        b.copy_from_slice(self.take(4)?);
        Ok(u32::from_be_bytes(b))
    }
    fn u64(&mut self) -> Result<u64, ArtifactError> {
        let mut b = [0; 8];
        b.copy_from_slice(self.take(8)?);
        Ok(u64::from_be_bytes(b))
    }
    fn u128(&mut self) -> Result<u128, ArtifactError> {
        let mut b = [0; 16];
        b.copy_from_slice(self.take(16)?);
        Ok(u128::from_be_bytes(b))
    }
    fn biguint(&mut self) -> Result<BigUint, ArtifactError> {
        let bytes = self.bytes_vec()?;
        if bytes.first() == Some(&0) {
            return Err(ArtifactError::InvalidPrior(
                "noncanonical leading zero in arbitrary-size integer",
            ));
        }
        Ok(BigUint::from_bytes_be(&bytes))
    }
    fn count(&mut self) -> Result<usize, ArtifactError> {
        Ok(self.u32()? as usize)
    }
    fn bytes_vec(&mut self) -> Result<Vec<u8>, ArtifactError> {
        let n = self.count()?;
        Ok(self.take(n)?.to_vec())
    }
    fn string(&mut self) -> Result<String, ArtifactError> {
        String::from_utf8(self.bytes_vec()?).map_err(|_| ArtifactError::InvalidUtf8)
    }
    fn strings(&mut self) -> Result<Vec<String>, ArtifactError> {
        let n = self.count()?;
        let mut rows = Vec::with_capacity(n);
        for _ in 0..n {
            rows.push(self.string()?);
        }
        Ok(rows)
    }
    fn optional_u32(&mut self) -> Result<Option<u32>, ArtifactError> {
        match self.u8()? {
            0 => Ok(None),
            1 => Ok(Some(self.u32()?)),
            tag => Err(ArtifactError::UnsupportedTag {
                what: "optional u32",
                tag,
            }),
        }
    }
    fn symbols(&mut self) -> Result<Vec<u32>, ArtifactError> {
        let n = self.count()?;
        let mut v = Vec::with_capacity(n);
        for _ in 0..n {
            v.push(self.u32()?);
        }
        Ok(v)
    }
    fn weight(&mut self) -> Result<WeightExpr, ArtifactError> {
        match self.u8()? {
            0 => Ok(WeightExpr::FixedPoint {
                units: self.biguint()?,
                fractional_bits: self.u32()?,
            }),
            tag => Err(ArtifactError::UnsupportedTag {
                what: "weight",
                tag,
            }),
        }
    }
    fn probe(&mut self) -> Result<ScalarProbe, ArtifactError> {
        let field = match self.u8()? {
            0 => FieldDescriptor::Standard(
                StandardFieldDescriptor::new(self.u32()?, self.u32()?)
                    .map_err(|_| ArtifactError::InvalidField)?,
            ),
            tag => return Err(ArtifactError::UnsupportedTag { what: "field", tag }),
        };
        Ok(ScalarProbe {
            field,
            point_rank: self.u128()?,
            hasse_order: self.u32()?,
        })
    }
    fn scheme_payload(&mut self) -> Result<Scheme, ArtifactError> {
        let capacity = self.u32()?;
        let codec = match self.u8()? {
            0 => ContentCodecDescriptor::BinaryMsb0,
            1 => ContentCodecDescriptor::CtmB9,
            tag => return Err(ArtifactError::UnsupportedTag { what: "codec", tag }),
        };
        let fallback = match self.u8()? {
            1 => FallbackDescriptor::DirectRadixV1,
            2 => {
                let codec = match self.u8()? {
                    0 => ContentCodecDescriptor::BinaryMsb0,
                    1 => ContentCodecDescriptor::CtmB9,
                    tag => return Err(ArtifactError::UnsupportedTag { what: "codec", tag }),
                };
                let field = StandardFieldDescriptor::new(self.u32()?, self.u32()?)
                    .map_err(|_| ArtifactError::InvalidField)?;
                let optimized_points = [self.u32()?, self.u32()?, self.u32()?, self.u32()?];
                FallbackDescriptor::StaticPolynomialV2 {
                    codec,
                    field,
                    optimized_points,
                }
            }
            tag => {
                return Err(ArtifactError::UnsupportedTag {
                    what: "fallback",
                    tag,
                });
            }
        };
        let root = self.optional_u32()?;
        let node_count = self.count()?;
        let mut nodes = Vec::with_capacity(node_count);
        for _ in 0..node_count {
            let probe_count = self.count()?;
            let mut probes = Vec::with_capacity(probe_count);
            for _ in 0..probe_count {
                probes.push(self.probe()?);
            }
            let branch_count = self.count()?;
            let mut branches = Vec::with_capacity(branch_count);
            for _ in 0..branch_count {
                branches.push((AtomId(self.u32()?), self.u32()?));
            }
            let default_child = self.optional_u32()?;
            nodes.push(DecisionNode {
                action: ProbeBundle::new(probes)
                    .map_err(|e| ArtifactError::InvalidScheme(SchemeError::Bundle(e)))?,
                branches,
                default_child,
            });
        }
        let scheme = Scheme {
            capacity,
            codec,
            root,
            nodes,
            fallback,
        };
        scheme.validate().map_err(ArtifactError::InvalidScheme)?;
        Ok(scheme)
    }
    fn transformations(&mut self) -> Result<Vec<PriorTransform>, ArtifactError> {
        let n = self.count()?;
        let mut rows = Vec::with_capacity(n);
        for _ in 0..n {
            rows.push(match self.u8()? {
                0 => PriorTransform::PairOrderCanonicalized,
                1 => PriorTransform::GlobalBinaryComplementAggregated,
                2 => PriorTransform::Named(self.string()?),
                tag => {
                    return Err(ArtifactError::UnsupportedTag {
                        what: "prior transformation",
                        tag,
                    });
                }
            });
        }
        Ok(rows)
    }
    fn prior_metadata(&mut self) -> Result<PriorMetadata, ArtifactError> {
        match self.u8()? {
            0 => Ok(PriorMetadata::CanonicalCtmB9D12 {
                provenance: self.string()?,
            }),
            2 => {
                let model_identity = self.string()?;
                let builder_identity = self.string()?;
                let budget = match self.u8()? {
                    0 => None,
                    1 => Some(self.string()?),
                    tag => {
                        return Err(ArtifactError::UnsupportedTag {
                            what: "external prior budget",
                            tag,
                        });
                    }
                };
                let uncertainty_note = self.string()?;
                Ok(PriorMetadata::ExternalFinite {
                    model_identity,
                    builder_identity,
                    budget,
                    uncertainty_note,
                })
            }
            tag => Err(ArtifactError::UnsupportedTag {
                what: "prior metadata",
                tag,
            }),
        }
    }
    fn provenance(&mut self) -> Result<Vec<ProvenanceRecord>, ArtifactError> {
        let n = self.count()?;
        let mut rows = Vec::with_capacity(n);
        for _ in 0..n {
            rows.push(ProvenanceRecord {
                name: self.string()?,
                source: self.string()?,
                license: self.string()?,
            });
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prior::ProvenanceRecord;

    #[test]
    fn scheme_v1_round_trip() {
        let scheme = Scheme::fallback_only(257, ContentCodecDescriptor::BinaryMsb0).unwrap();
        let zero = WeightExpr::FixedPoint {
            units: BigUint::from(0u8),
            fractional_bits: 0,
        };
        let artifact = SchemeArtifact {
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
            spec_revision: "v9".into(),
            cooker_revision: "1".into(),
        };
        let bytes = encode_scheme(&artifact).unwrap();
        assert_eq!(decode_scheme(&bytes).unwrap(), artifact);
    }

    #[test]
    fn checked_in_binary_artifacts_decode() {
        let scheme = include_bytes!("../data/canonical/teca-canonical-b9-d12-n12359.tecasm");
        let lexicon = include_bytes!(
            "../data/canonical/lexicon-cross-tokenizer-all-seven-alphanumeric-v1.tecalx"
        );
        assert!(decode_scheme(scheme).is_ok());
        assert!(decode_lexicon(lexicon).is_ok());
    }

    #[test]
    fn prior_round_trip_preserves_canonical_fixed_point_mass() {
        let prior = PriorArtifact {
            symbol_space: SymbolSpace {
                cardinality: 9,
                name: "CTM-B9".into(),
            },
            scenarios: vec![Scenario {
                left: vec![0],
                right: vec![1],
                weight: WeightExpr::FixedPoint {
                    units: 2u32.into(),
                    fractional_bits: 96,
                },
            }],
            transformations: vec![PriorTransform::PairOrderCanonicalized],
            metadata: PriorMetadata::CanonicalCtmB9D12 {
                provenance: "verified".into(),
            },
            provenance: vec![],
        };
        let bytes = encode_prior(&prior).unwrap();
        assert_eq!(decode_prior(&bytes).unwrap(), prior);
    }

    #[test]
    fn lexicon_round_trip_preserves_order() {
        let artifact = LexiconArtifact {
            lexicon: Lexicon::new(vec![b"a".to_vec(), b"bc".to_vec()]).unwrap(),
            renderer: "length-hex-v1".into(),
            profile: "test".into(),
            validation: vec!["distinct atoms".into()],
            tokenizer_families: vec!["test/tokenizer".into()],
            provenance: vec![ProvenanceRecord {
                name: "x".into(),
                source: "y".into(),
                license: "z".into(),
            }],
        };
        let bytes = encode_lexicon(&artifact).unwrap();
        assert_eq!(decode_lexicon(&bytes).unwrap(), artifact);
    }
    #[test]
    fn external_prior_metadata_round_trips() {
        let prior = PriorArtifact {
            symbol_space: SymbolSpace {
                cardinality: 2,
                name: "binary".into(),
            },
            scenarios: vec![Scenario {
                left: vec![0],
                right: vec![1],
                weight: WeightExpr::FixedPoint {
                    units: BigUint::from(3u8),
                    fractional_bits: 5,
                },
            }],
            transformations: vec![PriorTransform::PairOrderCanonicalized],
            metadata: PriorMetadata::ExternalFinite {
                model_identity: "hand-authored-binary-v1".into(),
                builder_identity: "fixture".into(),
                budget: Some("finite fixture".into()),
                uncertainty_note: String::new(),
            },
            provenance: vec![],
        };
        let bytes = encode_prior(&prior).unwrap();
        assert_eq!(decode_prior(&bytes).unwrap(), prior);
    }

    #[test]
    fn prior_v1_round_trip_preserves_arbitrary_precision_integers() {
        let huge = (BigUint::from(1u8) << 300usize) + BigUint::from(123u32);
        let prior = PriorArtifact {
            symbol_space: SymbolSpace {
                cardinality: 9,
                name: "CTM-B9".into(),
            },
            scenarios: vec![Scenario {
                left: vec![0],
                right: vec![1],
                weight: WeightExpr::FixedPoint {
                    units: huge.clone(),
                    fractional_bits: 64,
                },
            }],
            transformations: vec![PriorTransform::PairOrderCanonicalized],
            metadata: PriorMetadata::CanonicalCtmB9D12 {
                provenance: "test".into(),
            },
            provenance: vec![],
        };
        let bytes = encode_prior(&prior).unwrap();
        let decoded = decode_prior(&bytes).unwrap();
        assert_eq!(decoded, prior);
        let WeightExpr::FixedPoint { units, .. } = &decoded.scenarios[0].weight;
        assert_eq!(units, &huge);
    }
}
