use std::fmt;

/// Frozen content-to-symbol transform used by a cooked scheme.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentCodecDescriptor {
    /// Canonical binary source alphabet: bytes are emitted MSB-first.
    BinaryMsb0,
    /// Canonical CTM-B9 checkpoint: affine byte map then three base-9 digits.
    CtmB9,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodecError {
    InvalidPrime(u32),
    InvalidSymbolCardinality(u32),
    SymbolOutOfRange { symbol: u32, cardinality: u32 },
    CardinalityOverflow,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPrime(p) => write!(f, "invalid prime-field characteristic {p}"),
            Self::InvalidSymbolCardinality(n) => {
                write!(f, "source-symbol cardinality must be at least 2, got {n}")
            }
            Self::SymbolOutOfRange {
                symbol,
                cardinality,
            } => write!(
                f,
                "source symbol {symbol} is outside declared alphabet 0..{}",
                cardinality.saturating_sub(1)
            ),
            Self::CardinalityOverflow => write!(f, "codec cardinality calculation overflowed"),
        }
    }
}

impl std::error::Error for CodecError {}

impl ContentCodecDescriptor {
    pub const fn symbol_cardinality(self) -> u32 {
        match self {
            Self::BinaryMsb0 => 2,
            Self::CtmB9 => 9,
        }
    }

    pub fn symbols(self, bytes: &[u8]) -> Vec<u32> {
        match self {
            Self::BinaryMsb0 => {
                let mut out = Vec::with_capacity(bytes.len() * 8);
                for &byte in bytes {
                    for shift in (0..8).rev() {
                        out.push(((byte >> shift) & 1) as u32);
                    }
                }
                out
            }
            Self::CtmB9 => {
                let mut out = Vec::with_capacity(bytes.len() * 3);
                for &byte in bytes {
                    let v = (17u32 * byte as u32 + 17) % 729;
                    out.push(v / 81);
                    out.push((v / 9) % 9);
                    out.push(v % 9);
                }
                out
            }
        }
    }

    /// Convert source symbols to fixed-width positional base-p coefficients and
    /// append the monic length frame as the final high-degree coefficient.
    ///
    /// Returned coefficients are low degree first:
    /// `P(X) = coeffs[0] + coeffs[1]X + ...`.
    pub fn monic_prime_coefficients(self, bytes: &[u8], p: u32) -> Result<Vec<u32>, CodecError> {
        if p < 2 {
            return Err(CodecError::InvalidPrime(p));
        }
        if let Self::CtmB9 = self {
            let mut out = Vec::with_capacity(bytes.len() * 3 + 1);
            for &byte in bytes {
                let v = (17u32 * byte as u32 + 17) % 729;
                out.push(v / 81 + 1);
                out.push((v / 9) % 9 + 1);
                out.push(v % 9 + 1);
            }
            out.push(10);
            return Ok(out);
        }
        let symbols = self.symbols(bytes);
        monic_symbol_coefficients(&symbols, self.symbol_cardinality(), p)
    }
}

/// Convert an abstract source-symbol string to fixed-width positional base-p
/// coefficients and append the monic length frame. This is the cooker-facing
/// form; runtime byte codecs first produce their corresponding source symbols.
pub fn monic_symbol_coefficients(
    symbols: &[u32],
    symbol_cardinality: u32,
    p: u32,
) -> Result<Vec<u32>, CodecError> {
    if p < 2 {
        return Err(CodecError::InvalidPrime(p));
    }
    if symbol_cardinality < 2 {
        return Err(CodecError::InvalidSymbolCardinality(symbol_cardinality));
    }
    let width = digits_needed(symbol_cardinality, p)?;
    let capacity = symbols
        .len()
        .checked_mul(width)
        .and_then(|n| n.checked_add(1))
        .ok_or(CodecError::CardinalityOverflow)?;
    let mut out = Vec::with_capacity(capacity);
    for &symbol in symbols {
        if symbol >= symbol_cardinality {
            return Err(CodecError::SymbolOutOfRange {
                symbol,
                cardinality: symbol_cardinality,
            });
        }
        let mut digits = vec![0u32; width];
        let mut value = symbol;
        for slot in (0..width).rev() {
            digits[slot] = value % p;
            value /= p;
        }
        debug_assert_eq!(value, 0);
        out.extend(digits);
    }
    out.push(1);
    Ok(out)
}

fn digits_needed(cardinality: u32, base: u32) -> Result<usize, CodecError> {
    if cardinality < 2 {
        return Err(CodecError::InvalidSymbolCardinality(cardinality));
    }
    let mut width = 1usize;
    let mut capacity = base as u128;
    while capacity < cardinality as u128 {
        capacity = capacity
            .checked_mul(base as u128)
            .ok_or(CodecError::CardinalityOverflow)?;
        width += 1;
    }
    Ok(width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctm_b9_byte_codec_is_three_base9_digits() {
        let symbols = ContentCodecDescriptor::CtmB9.symbols(&[0, 255]);
        assert_eq!(symbols.len(), 6);
        assert!(symbols.iter().all(|&x| x < 9));
    }

    #[test]
    fn runtime_coefficients_match_canonical_ctm_framing() {
        assert_eq!(
            ContentCodecDescriptor::CtmB9
                .monic_prime_coefficients(&[0], 107)
                .unwrap(),
            vec![1, 2, 9, 10]
        );
    }

    #[test]
    fn binary_codec_is_msb_first() {
        assert_eq!(
            ContentCodecDescriptor::BinaryMsb0.symbols(&[0b1010_0001]),
            vec![1, 0, 1, 0, 0, 0, 0, 1]
        );
    }
}
