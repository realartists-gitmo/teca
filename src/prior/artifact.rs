use num_bigint::BigUint;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProvenanceRecord {
    pub name: String,
    pub source: String,
    pub license: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum PriorTransform {
    PairOrderCanonicalized,
    GlobalBinaryComplementAggregated,
    Named(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SymbolSpace {
    pub cardinality: u32,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WeightExpr {
    FixedPoint {
        units: BigUint,
        fractional_bits: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Scenario {
    pub left: Vec<u32>,
    pub right: Vec<u32>,
    pub weight: WeightExpr,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PriorMetadata {
    CanonicalCtmB9D12 {
        provenance: String,
    },
    ExternalFinite {
        model_identity: String,
        builder_identity: String,
        budget: Option<String>,
        uncertainty_note: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriorArtifact {
    pub symbol_space: SymbolSpace,
    pub scenarios: Vec<Scenario>,
    pub transformations: Vec<PriorTransform>,
    pub metadata: PriorMetadata,
    pub provenance: Vec<ProvenanceRecord>,
}

pub trait PairPrior {
    fn symbol_space(&self) -> &SymbolSpace;
    fn scenarios(&self) -> &[Scenario];
    fn transformations(&self) -> &[PriorTransform];
    fn metadata(&self) -> &PriorMetadata;
}

pub trait PriorBuilder {
    type Error: std::error::Error + Send + Sync + 'static;
    fn build(&self) -> Result<PriorArtifact, Self::Error>;
}

impl PairPrior for PriorArtifact {
    fn symbol_space(&self) -> &SymbolSpace {
        &self.symbol_space
    }
    fn scenarios(&self) -> &[Scenario] {
        &self.scenarios
    }
    fn transformations(&self) -> &[PriorTransform] {
        &self.transformations
    }
    fn metadata(&self) -> &PriorMetadata {
        &self.metadata
    }
}
