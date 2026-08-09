use std::fmt;

use crate::codec::{CodecError, ContentCodecDescriptor, monic_symbol_coefficients};
use crate::field::FieldError;
use crate::field::polynomial::evaluate_hasse;
use crate::field::standardff::{StandardFfError, StandardFieldDescriptor};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldDescriptor {
    Standard(StandardFieldDescriptor),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScalarProbe {
    pub field: FieldDescriptor,
    pub point_rank: u128,
    pub hasse_order: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProbeError {
    Codec(CodecError),
    Field(FieldError),
    StandardFf(StandardFfError),
    PointRankOutOfRange { point_rank: u128, cardinality: u128 },
}

impl fmt::Display for ProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Codec(err) => err.fmt(f),
            Self::Field(err) => err.fmt(f),
            Self::StandardFf(err) => err.fmt(f),
            Self::PointRankOutOfRange {
                point_rank,
                cardinality,
            } => {
                write!(
                    f,
                    "probe point rank {point_rank} is outside field cardinality {cardinality}"
                )
            }
        }
    }
}

impl std::error::Error for ProbeError {}

impl From<CodecError> for ProbeError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
    }
}
impl From<FieldError> for ProbeError {
    fn from(value: FieldError) -> Self {
        Self::Field(value)
    }
}
impl From<StandardFfError> for ProbeError {
    fn from(value: StandardFfError) -> Self {
        Self::StandardFf(value)
    }
}

impl ScalarProbe {
    pub fn field_cardinality(&self) -> Result<u128, ProbeError> {
        let cardinality = match &self.field {
            FieldDescriptor::Standard(desc) => desc.order()?,
        };
        if self.point_rank >= cardinality {
            return Err(ProbeError::PointRankOutOfRange {
                point_rank: self.point_rank,
                cardinality,
            });
        }
        Ok(cardinality)
    }

    pub fn validate(&self) -> Result<u128, ProbeError> {
        let cardinality = self.field_cardinality()?;
        let FieldDescriptor::Standard(desc) = &self.field;
        // Construction is part of the scheme ABI, so validate that the
        // standardized model is actually constructible before runtime.
        let _ = desc.instantiate()?;
        Ok(cardinality)
    }

    pub fn evaluate(
        &self,
        bytes: &[u8],
        codec: ContentCodecDescriptor,
    ) -> Result<(u128, u128), ProbeError> {
        self.validate()?;
        let field = match &self.field {
            FieldDescriptor::Standard(desc) => desc.instantiate()?,
        };
        let coefficients = codec.monic_prime_coefficients(bytes, field.characteristic())?;
        let point = field.from_rank(self.point_rank)?;
        let value = evaluate_hasse(&field, &coefficients, self.hasse_order as usize, &point)?;
        Ok((field.rank(&value)?, field.cardinality()))
    }

    pub fn evaluate_symbols(
        &self,
        symbols: &[u32],
        symbol_cardinality: u32,
    ) -> Result<(u128, u128), ProbeError> {
        self.validate()?;
        let field = match &self.field {
            FieldDescriptor::Standard(desc) => desc.instantiate()?,
        };
        let coefficients =
            monic_symbol_coefficients(symbols, symbol_cardinality, field.characteristic())?;
        let point = field.from_rank(self.point_rank)?;
        let value = evaluate_hasse(&field, &coefficients, self.hasse_order as usize, &point)?;
        Ok((field.rank(&value)?, field.cardinality()))
    }
}
