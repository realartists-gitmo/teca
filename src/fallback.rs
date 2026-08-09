use std::fmt;

use crate::address::AtomId;
use crate::codec::ContentCodecDescriptor;
use crate::field::standardff::StandardFieldDescriptor;
use crate::probe::{FieldDescriptor, ScalarProbe};

/// Universal exact continuation after the cooked structural tree has no custom
/// branch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FallbackDescriptor {
    DirectRadixV1,
    /// Infinite, corpus-independent Hasse-probe stream. The first four
    /// points are cooked for the active prior; the remainder is canonical
    /// ascending field-point order with those points removed.
    StaticPolynomialV2 {
        codec: ContentCodecDescriptor,
        field: StandardFieldDescriptor,
        optimized_points: [u32; 4],
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FallbackError {
    CapacityTooSmall(u32),
    CapacityMathOverflow,
    LengthOverflow,
    Infinite,
    InvalidStaticField,
    StaticCapacityTooSmall { field_order: u128, capacity: u32 },
}

impl fmt::Display for FallbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityTooSmall(n) => write!(f, "fallback requires capacity >= 2, got {n}"),
            Self::CapacityMathOverflow => write!(f, "fallback radix calculation overflowed"),
            Self::LengthOverflow => write!(f, "input length cannot be framed"),
            Self::Infinite => write!(f, "static polynomial fallback has no finite length"),
            Self::InvalidStaticField => write!(f, "static polynomial fallback field is invalid"),
            Self::StaticCapacityTooSmall {
                field_order,
                capacity,
            } => write!(
                f,
                "static polynomial fallback requires capacity >= field order {field_order}, got {capacity}"
            ),
        }
    }
}
impl std::error::Error for FallbackError {}

impl FallbackDescriptor {
    pub fn address<'a>(
        self,
        bytes: &'a [u8],
        capacity: u32,
    ) -> Result<FallbackAddress<'a>, FallbackError> {
        match self {
            Self::DirectRadixV1 => FallbackAddress::new(bytes, capacity),
            Self::StaticPolynomialV2 {
                codec,
                field,
                optimized_points,
            } => FallbackAddress::new_static(bytes, capacity, codec, field, optimized_points),
        }
    }

    pub fn validate(self, capacity: u32) -> Result<(), FallbackError> {
        if let Self::StaticPolynomialV2 {
            field,
            optimized_points,
            ..
        } = self
        {
            let order = field
                .order()
                .map_err(|_| FallbackError::InvalidStaticField)?;
            if order > capacity as u128 {
                return Err(FallbackError::StaticCapacityTooSmall {
                    field_order: order,
                    capacity,
                });
            }
            if optimized_points
                .iter()
                .any(|&p| p == 0 || p as u128 >= order)
                || (0..4).any(|i| ((i + 1)..4).any(|j| optimized_points[i] == optimized_points[j]))
            {
                return Err(FallbackError::InvalidStaticField);
            }
        }
        Ok(())
    }
}

// 15 bytes = 120 bits, leaving enough headroom to perform every canonical
// radix calculation in u128 while making fixed-block rounding loss small.
const BLOCK_BYTES: usize = 15;

pub struct FallbackAddress<'a> {
    state: FallbackState<'a>,
}

enum FallbackState<'a> {
    V1(DirectRadixV1State<'a>),
    Static(StaticPolynomialState<'a>),
}

struct StaticPolynomialState<'a> {
    bytes: &'a [u8],
    codec: ContentCodecDescriptor,
    field: StandardFieldDescriptor,
    optimized_points: [u32; 4],
    depth: usize,
}

struct GammaState {
    value: usize,
    zero_remaining: usize,
    bit_remaining: usize,
}

impl GammaState {
    fn new(byte_len: usize) -> Result<Self, FallbackError> {
        let value = byte_len
            .checked_add(1)
            .ok_or(FallbackError::LengthOverflow)?;
        let bits = usize::BITS as usize - value.leading_zeros() as usize;
        Ok(Self {
            value,
            zero_remaining: bits - 1,
            bit_remaining: bits,
        })
    }

    fn next(&mut self) -> Option<AtomId> {
        if self.zero_remaining != 0 {
            self.zero_remaining -= 1;
            return Some(AtomId(0));
        }
        if self.bit_remaining != 0 {
            let shift = self.bit_remaining - 1;
            self.bit_remaining -= 1;
            return Some(AtomId(((self.value >> shift) & 1) as u32));
        }
        None
    }

    fn encoded_len(&self) -> Result<usize, FallbackError> {
        let bits = usize::BITS as usize - self.value.leading_zeros() as usize;
        bits.checked_mul(2)
            .and_then(|n| n.checked_sub(1))
            .ok_or(FallbackError::LengthOverflow)
    }
}

/// Canonical V1: Elias-gamma byte length, capacity-radix 15-byte blocks, zero tail.
struct DirectRadixV1State<'a> {
    bytes: &'a [u8],
    base: u32,
    gamma: GammaState,
    byte_index: usize,
    block_value: u128,
    block_divisor: u128,
    block_digits_remaining: usize,
}

impl<'a> FallbackAddress<'a> {
    pub fn new(bytes: &'a [u8], capacity: u32) -> Result<Self, FallbackError> {
        if capacity < 2 {
            return Err(FallbackError::CapacityTooSmall(capacity));
        }
        Ok(Self {
            state: FallbackState::V1(DirectRadixV1State {
                bytes,
                base: capacity,
                gamma: GammaState::new(bytes.len())?,
                byte_index: 0,
                block_value: 0,
                block_divisor: 1,
                block_digits_remaining: 0,
            }),
        })
    }

    pub fn new_static(
        bytes: &'a [u8],
        capacity: u32,
        codec: ContentCodecDescriptor,
        field: StandardFieldDescriptor,
        optimized_points: [u32; 4],
    ) -> Result<Self, FallbackError> {
        let descriptor = FallbackDescriptor::StaticPolynomialV2 {
            codec,
            field,
            optimized_points,
        };
        descriptor.validate(capacity)?;
        Ok(Self {
            state: FallbackState::Static(StaticPolynomialState {
                bytes,
                codec,
                field,
                optimized_points,
                depth: 0,
            }),
        })
    }

    pub fn next_atom(&mut self) -> AtomId {
        match &mut self.state {
            FallbackState::V1(state) => next_v1(state),
            FallbackState::Static(state) => next_static(state),
        }
    }

    pub fn finite_len(&self) -> Result<usize, FallbackError> {
        match &self.state {
            FallbackState::V1(state) => direct_radix_v1_finite_len(state.bytes.len(), state.base),
            FallbackState::Static(_) => Err(FallbackError::Infinite),
        }
    }

    #[cfg(test)]
    fn finite_prefix_vec(mut self) -> Vec<AtomId> {
        let n = self.finite_len().expect("test input length fits usize");
        (0..n).map(|_| self.next_atom()).collect()
    }
}

fn next_static(state: &mut StaticPolynomialState<'_>) -> AtomId {
    let order = state.field.order().expect("validated static field") as usize;
    let point_count = order - 1;
    let (_hasse_order, slot) = (state.depth / point_count, state.depth % point_count);
    let point = if slot < state.optimized_points.len() {
        state.optimized_points[slot]
    } else {
        let mut candidate = (slot - state.optimized_points.len() + 1) as u32;
        for &blocked in &state.optimized_points {
            if candidate >= blocked {
                candidate += 1;
            }
        }
        candidate
    };
    let probe = ScalarProbe {
        field: FieldDescriptor::Standard(state.field),
        point_rank: point as u128,
        hasse_order: (state.depth / point_count) as u32,
    };
    let value = probe
        .evaluate(state.bytes, state.codec)
        .expect("validated static probe stream must evaluate")
        .0;
    state.depth += 1;
    AtomId(value as u32)
}

fn next_v1(state: &mut DirectRadixV1State<'_>) -> AtomId {
    if let Some(atom) = state.gamma.next() {
        return atom;
    }
    if state.block_digits_remaining == 0 && state.byte_index < state.bytes.len() {
        load_v1_block(state).expect("validated direct-radix block geometry cannot overflow");
    }
    if state.block_digits_remaining != 0 {
        let digit = state.block_value / state.block_divisor;
        state.block_value %= state.block_divisor;
        state.block_digits_remaining -= 1;
        if state.block_digits_remaining != 0 {
            state.block_divisor /= state.base as u128;
        }
        debug_assert!(digit < state.base as u128);
        return AtomId(digit as u32);
    }
    AtomId(0)
}

fn load_v1_block(state: &mut DirectRadixV1State<'_>) -> Result<(), FallbackError> {
    let end = state
        .byte_index
        .saturating_add(BLOCK_BYTES)
        .min(state.bytes.len());
    let block = &state.bytes[state.byte_index..end];
    let mut value = 0u128;
    for &byte in block {
        value = value
            .checked_mul(256)
            .and_then(|x| x.checked_add(byte as u128))
            .ok_or(FallbackError::CapacityMathOverflow)?;
    }
    let digits = radix_digits_for_bytes(state.base, block.len())?;
    let divisor =
        pow_u128(state.base as u128, digits - 1).ok_or(FallbackError::CapacityMathOverflow)?;
    state.byte_index = end;
    state.block_value = value;
    state.block_divisor = divisor;
    state.block_digits_remaining = digits;
    Ok(())
}

/// Minimum d such that N^d >= 256^bytes. `bytes <= BLOCK_BYTES`, so N^(d-1)
/// remains below the target and therefore fits u128.
fn radix_digits_for_bytes(base: u32, bytes: usize) -> Result<usize, FallbackError> {
    debug_assert!(bytes <= BLOCK_BYTES);
    if bytes == 0 {
        return Ok(0);
    }
    let target = 1u128
        .checked_shl((bytes * 8) as u32)
        .ok_or(FallbackError::CapacityMathOverflow)?;
    radix_digits_for_target(base, target)
}

fn radix_digits_for_target(base: u32, target: u128) -> Result<usize, FallbackError> {
    let mut represented = 1u128;
    let mut digits = 0usize;
    while represented < target {
        digits = digits
            .checked_add(1)
            .ok_or(FallbackError::CapacityMathOverflow)?;
        match represented.checked_mul(base as u128) {
            Some(next) => represented = next,
            None => break,
        }
    }
    Ok(digits)
}

/// Exact number of non-tail atoms emitted by the canonical DirectRadixV1
/// fallback for an input of `byte_len` bytes. This is allocation-free and is
/// used by the conformance/benchmark harness.
pub fn direct_radix_v1_finite_len(byte_len: usize, capacity: u32) -> Result<usize, FallbackError> {
    if capacity < 2 {
        return Err(FallbackError::CapacityTooSmall(capacity));
    }
    let gamma = GammaState::new(byte_len)?.encoded_len()?;
    let full_blocks = byte_len / BLOCK_BYTES;
    let remainder = byte_len % BLOCK_BYTES;
    let full_width = radix_digits_for_bytes(capacity, BLOCK_BYTES)?;
    let remainder_width = if remainder == 0 {
        0
    } else {
        radix_digits_for_bytes(capacity, remainder)?
    };
    let payload = full_blocks
        .checked_mul(full_width)
        .and_then(|n| n.checked_add(remainder_width))
        .ok_or(FallbackError::LengthOverflow)?;
    gamma
        .checked_add(payload)
        .ok_or(FallbackError::LengthOverflow)
}

/// Reference finite length for a bundled binary-Hasse-at-zero tail. A bundle
/// carries `floor(log2(capacity))` coefficient bits; the monic coefficient is
/// the exact end marker, so the finite informative stream contains
/// `8*byte_len + 1` bits. This is a comparison oracle, not the canonical
/// runtime fallback.
pub fn binary_hasse_zero_finite_len(
    byte_len: usize,
    capacity: u32,
) -> Result<usize, FallbackError> {
    if capacity < 2 {
        return Err(FallbackError::CapacityTooSmall(capacity));
    }
    let bits_per_atom = (u32::BITS - 1 - capacity.leading_zeros()) as usize;
    debug_assert!(bits_per_atom >= 1);
    let bits = byte_len
        .checked_mul(8)
        .and_then(|n| n.checked_add(1))
        .ok_or(FallbackError::LengthOverflow)?;
    bits.checked_add(bits_per_atom - 1)
        .map(|n| n / bits_per_atom)
        .ok_or(FallbackError::LengthOverflow)
}

fn pow_u128(mut base: u128, mut exponent: usize) -> Option<u128> {
    let mut out = 1u128;
    while exponent != 0 {
        if exponent & 1 != 0 {
            out = out.checked_mul(base)?;
        }
        exponent >>= 1;
        if exponent != 0 {
            base = base.checked_mul(base)?;
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_inputs_diverge() {
        let mut a = FallbackAddress::new(b"a", 2).unwrap();
        let mut b = FallbackAddress::new(b"b", 2).unwrap();
        assert!((0..64).any(|_| a.next_atom() != b.next_atom()));
    }

    #[test]
    fn length_is_part_of_prefix_code() {
        let a = FallbackAddress::new(b"", 257).unwrap().finite_prefix_vec();
        let b = FallbackAddress::new(&[0], 257).unwrap().finite_prefix_vec();
        assert_ne!(a, b);
        assert!(!a.starts_with(&b));
        assert!(!b.starts_with(&a));
    }

    #[test]
    fn full_blocks_use_capacity_efficient_widths() {
        assert_eq!(radix_digits_for_bytes(2, 15).unwrap(), 120);
        assert_eq!(radix_digits_for_bytes(3, 15).unwrap(), 76);
        assert_eq!(radix_digits_for_bytes(257, 15).unwrap(), 15);
        assert_eq!(radix_digits_for_bytes(11_456, 15).unwrap(), 9);
    }

    #[test]
    fn all_one_byte_inputs_are_distinct_for_representative_capacities() {
        for capacity in [2, 3, 11, 257, 11_456] {
            let codes: std::collections::BTreeSet<Vec<_>> = (0u8..=255)
                .map(|b| {
                    FallbackAddress::new(&[b], capacity)
                        .unwrap()
                        .finite_prefix_vec()
                })
                .collect();
            assert_eq!(codes.len(), 256);
        }
    }

    #[test]
    fn leading_zeroes_are_preserved_by_fixed_block_width() {
        for capacity in [2, 3, 257, 11_456] {
            let a = FallbackAddress::new(&[0, 1], capacity)
                .unwrap()
                .finite_prefix_vec();
            let b = FallbackAddress::new(&[1], capacity)
                .unwrap()
                .finite_prefix_vec();
            assert_ne!(a, b);
        }
    }

    #[test]
    fn finite_length_helper_matches_iterator_geometry() {
        for capacity in [2, 3, 11, 257, 11_456] {
            for len in [0usize, 1, 2, 14, 15, 16, 31, 255, 4096] {
                let bytes = vec![0x5a; len];
                let address = FallbackAddress::new(&bytes, capacity).unwrap();
                assert_eq!(
                    address.finite_len().unwrap(),
                    direct_radix_v1_finite_len(len, capacity).unwrap()
                );
            }
        }
    }

    #[test]
    fn binary_hasse_reference_uses_largest_binary_bundle() {
        assert_eq!(binary_hasse_zero_finite_len(0, 2).unwrap(), 1);
        assert_eq!(binary_hasse_zero_finite_len(1, 2).unwrap(), 9);
        assert_eq!(binary_hasse_zero_finite_len(15, 11_456).unwrap(), 10);
        assert_eq!(binary_hasse_zero_finite_len(1500, 11_456).unwrap(), 924);
        assert!(
            direct_radix_v1_finite_len(1500, 11_456).unwrap()
                < binary_hasse_zero_finite_len(1500, 11_456).unwrap()
        );
    }

    #[test]
    fn static_polynomial_stream_is_infinite_and_matches_probe() {
        let field = StandardFieldDescriptor {
            characteristic: 12_347,
            degree: 1,
        };
        let mut address = FallbackAddress::new_static(
            b"Gortnite",
            12_359,
            ContentCodecDescriptor::CtmB9,
            field,
            [6743, 2728, 6148, 1809],
        )
        .unwrap();
        let expected = ScalarProbe {
            field: FieldDescriptor::Standard(field),
            point_rank: 6743,
            hasse_order: 0,
        }
        .evaluate(b"Gortnite", ContentCodecDescriptor::CtmB9)
        .unwrap()
        .0;
        assert_eq!(address.next_atom(), AtomId(expected as u32));
        assert_eq!(address.finite_len(), Err(FallbackError::Infinite));
    }
}
