//! Lübeck StandardFF construction for TECA finite fields.
//!
//! This is a clean Rust transcription of the standardized mathematical
//! construction used by Frank Lübeck's GAP `StandardFF` package. TECA keeps the
//! natural tower basis directly instead of converting it to GAP's faster simple
//! extension representation; the tower basis is the authority for Steinitz
//! numbering and canonical embeddings.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};

use super::{ExplicitField, FieldElement, FieldError};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StandardFieldDescriptor {
    pub characteristic: u32,
    pub degree: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StandardFfError {
    InvalidDescriptor,
    Arithmetic(FieldError),
    IntegerOverflow,
    CachePoisoned,
    InternalConstruction(&'static str),
}

impl fmt::Display for StandardFfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDescriptor => write!(f, "invalid StandardFF descriptor"),
            Self::Arithmetic(err) => err.fmt(f),
            Self::IntegerOverflow => write!(f, "StandardFF integer construction overflowed u128"),
            Self::CachePoisoned => write!(
                f,
                "StandardFF construction cache is unavailable after a poisoned mutex"
            ),
            Self::InternalConstruction(msg) => write!(f, "StandardFF construction failed: {msg}"),
        }
    }
}

impl std::error::Error for StandardFfError {}

impl From<FieldError> for StandardFfError {
    fn from(value: FieldError) -> Self {
        Self::Arithmetic(value)
    }
}

impl StandardFieldDescriptor {
    pub fn new(characteristic: u32, degree: u32) -> Result<Self, StandardFfError> {
        let descriptor = Self {
            characteristic,
            degree,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    pub fn validate(self) -> Result<(), StandardFfError> {
        if self.degree == 0 || !is_prime_u32(self.characteristic) {
            return Err(StandardFfError::InvalidDescriptor);
        }
        checked_pow(self.characteristic as u128, self.degree)
            .ok_or(StandardFfError::IntegerOverflow)?;
        Ok(())
    }

    pub fn order(self) -> Result<u128, StandardFfError> {
        self.validate()?;
        checked_pow(self.characteristic as u128, self.degree)
            .ok_or(StandardFfError::IntegerOverflow)
    }

    pub fn instantiate(self) -> Result<ExplicitField, StandardFfError> {
        self.validate()?;
        standard_field(self.characteristic, self.degree)
    }
}

static FIELD_CACHE: OnceLock<Mutex<BTreeMap<(u32, u32), ExplicitField>>> = OnceLock::new();

fn standard_field(p: u32, n: u32) -> Result<ExplicitField, StandardFfError> {
    let key = (p, n);
    if let Some(field) = FIELD_CACHE
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| StandardFfError::CachePoisoned)?
        .get(&key)
        .cloned()
    {
        return Ok(field);
    }

    let field = if n == 1 {
        ExplicitField::prime(p)?
    } else {
        let factors = factorization(n);
        let &(r, k) = factors
            .last()
            .ok_or(StandardFfError::InternalConstruction("empty factorization"))?;
        let n1 = checked_pow(r as u128, k - 1)
            .and_then(|x| u32::try_from(x).ok())
            .ok_or(StandardFfError::IntegerOverflow)?;
        let base_degree = n / r;
        let coefficient_ranks = standard_prime_degree_coefficients(p, r, k)?;
        let embedded: Result<Vec<_>, _> = coefficient_ranks
            .into_iter()
            .map(|rank| embed_steinitz(p, n1, base_degree, rank))
            .collect();
        ExplicitField::tower(standard_field(p, base_degree)?, r as usize, &embedded?)?
    };

    FIELD_CACHE
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| StandardFfError::CachePoisoned)?
        .insert(key, field.clone());
    Ok(field)
}

/// Low coefficients of Lübeck's standard monic prime-degree polynomial for the
/// k-th r-extension over FF(p, r^(k-1)), represented by Steinitz numbers in the
/// base field. The leading coefficient 1 is implicit.
fn standard_prime_degree_coefficients(
    p: u32,
    r: u32,
    k: u32,
) -> Result<Vec<u128>, StandardFfError> {
    let base_degree = checked_pow(r as u128, k - 1)
        .and_then(|x| u32::try_from(x).ok())
        .ok_or(StandardFfError::IntegerOverflow)?;
    let base = standard_field(p, base_degree)?;
    let q = base.cardinality();

    let mut coefficients = vec![0u128; r as usize];
    if p == r {
        // Artin-Schreier: X^p - X - prod_{i<k} x_i^(p-1).
        let constant_rank = (p as u128 - 1)
            .checked_mul(q / p as u128)
            .ok_or(StandardFfError::IntegerOverflow)?;
        coefficients[0] = constant_rank;
        if r > 1 {
            coefficients[1] = p as u128 - 1;
        }
        return Ok(coefficients);
    }

    if r == 2 && p % 4 == 3 {
        if k == 1 {
            coefficients[0] = 1; // X^2 + 1
            return Ok(coefficients);
        }
        if k == 2 {
            let f = standard_field(p, 2)?;
            let one = f.one();
            let exponent = (f.cardinality() - 1) / 2;
            let mut i = 1u128;
            loop {
                let rank = standard_affine_shift(f.cardinality(), i);
                if rank != 0 {
                    let x = f.from_rank(rank)?;
                    if f.pow(&x, exponent)? != one {
                        coefficients[0] = f.rank(&f.neg(&x)?)?;
                        return Ok(coefficients);
                    }
                }
                i = i.checked_add(1).ok_or(StandardFfError::IntegerOverflow)?;
            }
        }
        let previous_generator_rank = checked_pow(
            p as u128,
            checked_pow(r as u128, k - 2)
                .and_then(|x| u32::try_from(x).ok())
                .ok_or(StandardFfError::IntegerOverflow)?,
        )
        .ok_or(StandardFfError::IntegerOverflow)?;
        let x = base.from_rank(previous_generator_rank)?;
        coefficients[0] = base.rank(&base.neg(&x)?)?;
        return Ok(coefficients);
    }

    if (p - 1).is_multiple_of(r) {
        if k == 1 {
            let mut i = 1u128;
            loop {
                let nr = standard_affine_shift(p as u128, i) as u32;
                if nr != 0 && pow_mod_u32(nr, (p - 1) / r, p) != 1 {
                    coefficients[0] = (p - nr) as u128;
                    return Ok(coefficients);
                }
                i = i.checked_add(1).ok_or(StandardFfError::IntegerOverflow)?;
            }
        }
        let previous_degree = checked_pow(r as u128, k - 2)
            .and_then(|x| u32::try_from(x).ok())
            .ok_or(StandardFfError::IntegerOverflow)?;
        let previous_generator_rank =
            checked_pow(p as u128, previous_degree).ok_or(StandardFfError::IntegerOverflow)?;
        let x = base.from_rank(previous_generator_rank)?;
        coefficients[0] = base.rank(&base.neg(&x)?)?;
        return Ok(coefficients);
    }

    // General case: Algorithm 5.5 from Lübeck/StandardFF. Constant term is -1
    // at the first r-extension and -x_{r,k-1} thereafter.
    let constant = if k == 1 {
        base.neg(&base.one())?
    } else {
        let previous_degree = checked_pow(r as u128, k - 2)
            .and_then(|x| u32::try_from(x).ok())
            .ok_or(StandardFfError::IntegerOverflow)?;
        let rank =
            checked_pow(p as u128, previous_degree).ok_or(StandardFfError::IntegerOverflow)?;
        base.neg(&base.from_rank(rank)?)?
    };
    standard_irreducible_coefficients(&base, r as usize, constant)
}

fn standard_irreducible_coefficients(
    base: &ExplicitField,
    degree: usize,
    constant: FieldElement,
) -> Result<Vec<u128>, StandardFfError> {
    if degree < 2 {
        return Err(StandardFfError::InternalConstruction(
            "prime extension degree < 2",
        ));
    }
    let mut coefficients = vec![base.zero(); degree + 1];
    coefficients[0] = constant;
    coefficients[1] = base.one();
    coefficients[degree] = base.one();
    let q = base.cardinality();

    let mut inc = 1usize;
    while checked_pow(q, inc as u32).ok_or(StandardFfError::IntegerOverflow)? < 2 * degree as u128 {
        inc += 1;
    }
    let mut d = 0usize;
    let mut count = 0u128;
    while !is_irreducible_prime_degree(base, &coefficients)? {
        if count.is_multiple_of(degree as u128) && d < degree - 1 {
            d = (d + inc).min(degree - 1);
        }
        let coefficient_digits = d
            .checked_sub(1)
            .ok_or(StandardFfError::InternalConstruction(
                "StandardFF sparse-search degree did not advance",
            ))?;
        let qq =
            checked_pow(q, coefficient_digits as u32).ok_or(StandardFfError::IntegerOverflow)?;
        let shifted = standard_affine_shift(qq, count);
        let digits = qadic_digits(shifted, q, coefficient_digits);
        for index in 1..d {
            coefficients[index] = base.from_rank(digits[index - 1])?;
        }
        count = count
            .checked_add(1)
            .ok_or(StandardFfError::IntegerOverflow)?;
    }
    coefficients
        .into_iter()
        .take(degree)
        .map(|x| base.rank(&x).map_err(StandardFfError::from))
        .collect()
}

/// Prime-degree irreducibility criterion used by StandardFF: gcd(f, X^q-X)=1
/// and X^(q^r) == X mod f.
fn is_irreducible_prime_degree(
    field: &ExplicitField,
    polynomial: &[FieldElement],
) -> Result<bool, StandardFfError> {
    let degree = polynomial.len() - 1;
    let x = vec![field.zero(), field.one()];
    let xq = poly_pow_mod(field, &x, field.cardinality(), polynomial)?;
    let xq_minus_x = poly_sub(field, &xq, &x)?;
    let gcd = poly_gcd(field, polynomial.to_vec(), xq_minus_x)?;
    if poly_degree(&gcd) != 0 {
        return Ok(false);
    }
    let exponent =
        checked_pow(field.cardinality(), degree as u32).ok_or(StandardFfError::IntegerOverflow)?;
    Ok(poly_trim(poly_pow_mod(field, &x, exponent, polynomial)?) == poly_trim(x))
}

fn poly_pow_mod(
    field: &ExplicitField,
    base: &[FieldElement],
    mut exponent: u128,
    modulus: &[FieldElement],
) -> Result<Vec<FieldElement>, StandardFfError> {
    let mut out = vec![field.one()];
    let mut cur = poly_mod(field, base.to_vec(), modulus)?;
    while exponent != 0 {
        if exponent & 1 != 0 {
            out = poly_mod(field, poly_mul(field, &out, &cur)?, modulus)?;
        }
        exponent >>= 1;
        if exponent != 0 {
            cur = poly_mod(field, poly_mul(field, &cur, &cur)?, modulus)?;
        }
    }
    Ok(poly_trim(out))
}

fn poly_mul(
    field: &ExplicitField,
    a: &[FieldElement],
    b: &[FieldElement],
) -> Result<Vec<FieldElement>, StandardFfError> {
    if a.is_empty() || b.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = vec![field.zero(); a.len() + b.len() - 1];
    for (i, x) in a.iter().enumerate() {
        for (j, y) in b.iter().enumerate() {
            let product = field.mul(x, y)?;
            out[i + j] = field.add(&out[i + j], &product)?;
        }
    }
    Ok(poly_trim(out))
}

fn poly_sub(
    field: &ExplicitField,
    a: &[FieldElement],
    b: &[FieldElement],
) -> Result<Vec<FieldElement>, StandardFfError> {
    let mut out = vec![field.zero(); a.len().max(b.len())];
    for (i, slot) in out.iter_mut().enumerate() {
        let av = a.get(i).cloned().unwrap_or_else(|| field.zero());
        let bv = b.get(i).cloned().unwrap_or_else(|| field.zero());
        *slot = field.sub(&av, &bv)?;
    }
    Ok(poly_trim(out))
}

fn poly_mod(
    field: &ExplicitField,
    mut dividend: Vec<FieldElement>,
    modulus: &[FieldElement],
) -> Result<Vec<FieldElement>, StandardFfError> {
    dividend = poly_trim(dividend);
    let modulus = poly_trim(modulus.to_vec());
    if modulus.is_empty() {
        return Err(StandardFfError::InternalConstruction(
            "zero polynomial modulus",
        ));
    }
    let md = modulus.len() - 1;
    let lead_inv = field.inverse(&modulus[md])?;
    while !dividend.is_empty() && dividend.len() > md {
        let dd = dividend.len() - 1;
        let factor = field.mul(&dividend[dd], &lead_inv)?;
        let shift = dd - md;
        for (i, coefficient) in modulus.iter().enumerate() {
            let product = field.mul(&factor, coefficient)?;
            dividend[shift + i] = field.sub(&dividend[shift + i], &product)?;
        }
        dividend = poly_trim(dividend);
    }
    Ok(dividend)
}

fn poly_gcd(
    field: &ExplicitField,
    mut a: Vec<FieldElement>,
    mut b: Vec<FieldElement>,
) -> Result<Vec<FieldElement>, StandardFfError> {
    a = poly_trim(a);
    b = poly_trim(b);
    while !b.is_empty() {
        let r = poly_mod(field, a, &b)?;
        a = b;
        b = r;
    }
    if a.is_empty() {
        return Ok(a);
    }
    let inv = field.inverse(a.last().expect("nonempty"))?;
    for coefficient in &mut a {
        *coefficient = field.mul(coefficient, &inv)?;
    }
    Ok(poly_trim(a))
}

fn poly_degree(polynomial: &[FieldElement]) -> isize {
    polynomial.len() as isize - 1
}

fn poly_trim(mut polynomial: Vec<FieldElement>) -> Vec<FieldElement> {
    while polynomial.last().is_some_and(FieldElement::is_zero) {
        polynomial.pop();
    }
    polynomial
}

/// Lübeck's deterministic affine permutation of [0,q).
pub fn standard_affine_shift(q: u128, i: u128) -> u128 {
    if q <= 1 {
        return 0;
    }
    let mut multiplier = (q / 5) * 4 + ((q % 5) * 4) / 5;
    while gcd_u128(multiplier, q) != 1 {
        multiplier -= 1;
    }
    let addend = (q / 3) * 2 + ((q % 3) * 2) / 3;
    (mul_mod_u128(multiplier, i % q, q) + addend) % q
}

fn mul_mod_u128(mut a: u128, mut b: u128, modulus: u128) -> u128 {
    let mut out = 0u128;
    a %= modulus;
    while b != 0 {
        if b & 1 != 0 {
            out = if out >= modulus - a {
                out - (modulus - a)
            } else {
                out + a
            };
        }
        b >>= 1;
        if b != 0 {
            a = if a >= modulus - a {
                a - (modulus - a)
            } else {
                a + a
            };
        }
    }
    out
}

/// Canonical StandardFF embedding of a Steinitz number from FF(p,n) into
/// FF(p,m), n|m, expressed as the destination Steinitz number.
pub fn embed_steinitz(p: u32, n: u32, m: u32, rank: u128) -> Result<u128, StandardFfError> {
    if n == 0 || m == 0 || !m.is_multiple_of(n) {
        return Err(StandardFfError::InvalidDescriptor);
    }
    let source_size = checked_pow(p as u128, n).ok_or(StandardFfError::IntegerOverflow)?;
    if rank >= source_size {
        return Err(StandardFfError::Arithmetic(FieldError::RankOutOfRange {
            rank,
            cardinality: source_size,
        }));
    }
    if n == m || rank == 0 {
        return Ok(rank);
    }
    let digits = qadic_digits(rank, p as u128, n as usize);
    let degrees = std_mon_degrees(m);
    let map: Vec<usize> = degrees
        .iter()
        .enumerate()
        .filter_map(|(index, degree)| n.is_multiple_of(*degree).then_some(index))
        .collect();
    if map.len() < digits.len() {
        return Err(StandardFfError::InternalConstruction(
            "invalid StandardFF monomial embedding",
        ));
    }
    let mut output_digits = vec![0u128; m as usize];
    for (digit, &position) in digits.iter().zip(&map) {
        output_digits[position] = *digit;
    }
    value_base(&output_digits, p as u128)
}

/// Degrees over F_p of each tower-basis monomial, in Steinitz digit order.
pub fn std_mon_degrees(n: u32) -> Vec<u32> {
    if n == 1 {
        return vec![1];
    }
    let factors = factorization(n);
    let &(r, k) = factors.last().expect("n>1 has factor");
    let base_n = n / r;
    let mut result = std_mon_degrees(base_n);
    let extension_degree = r.pow(k);
    let lifted: Vec<u32> = result
        .iter()
        .map(|&degree| lcm_u32(degree, extension_degree))
        .collect();
    for _ in 1..r {
        result.extend_from_slice(&lifted);
    }
    result
}

fn factorization(mut n: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut p = 2u32;
    while p as u64 * p as u64 <= n as u64 {
        if n.is_multiple_of(p) {
            let mut exponent = 0;
            while n.is_multiple_of(p) {
                n /= p;
                exponent += 1;
            }
            out.push((p, exponent));
        }
        p += if p == 2 { 1 } else { 2 };
    }
    if n > 1 {
        out.push((n, 1));
    }
    out
}

fn qadic_digits(mut value: u128, base: u128, width: usize) -> Vec<u128> {
    let mut out = Vec::with_capacity(width);
    for _ in 0..width {
        out.push(if base <= 1 { 0 } else { value % base });
        if base > 1 {
            value /= base;
        }
    }
    out
}

fn value_base(digits: &[u128], base: u128) -> Result<u128, StandardFfError> {
    let mut place = 1u128;
    let mut out = 0u128;
    for (index, &digit) in digits.iter().enumerate() {
        out = out
            .checked_add(
                digit
                    .checked_mul(place)
                    .ok_or(StandardFfError::IntegerOverflow)?,
            )
            .ok_or(StandardFfError::IntegerOverflow)?;
        if index + 1 != digits.len() {
            place = place
                .checked_mul(base)
                .ok_or(StandardFfError::IntegerOverflow)?;
        }
    }
    Ok(out)
}

fn checked_pow(mut base: u128, mut exponent: u32) -> Option<u128> {
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

fn pow_mod_u32(base: u32, mut exponent: u32, modulus: u32) -> u32 {
    let mut out = 1u64;
    let mut cur = (base % modulus) as u64;
    let m = modulus as u64;
    while exponent != 0 {
        if exponent & 1 != 0 {
            out = out * cur % m;
        }
        exponent >>= 1;
        if exponent != 0 {
            cur = cur * cur % m;
        }
    }
    out as u32
}

fn gcd_u128(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

fn gcd_u32(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let r = a % b;
        a = b;
        b = r;
    }
    a
}

fn lcm_u32(a: u32, b: u32) -> u32 {
    a / gcd_u32(a, b) * b
}

fn is_prime_u32(n: u32) -> bool {
    if n < 2 {
        return false;
    }
    if n.is_multiple_of(2) {
        return n == 2;
    }
    let mut d = 3u32;
    while (d as u64) * (d as u64) <= n as u64 {
        if n.is_multiple_of(d) {
            return false;
        }
        d += 2;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_affine_shift_matches_gap_documentation() {
        let got: Vec<u128> = (0..=10).map(|i| standard_affine_shift(11, i)).collect();
        assert_eq!(got, vec![7, 4, 1, 9, 6, 3, 0, 8, 5, 2, 10]);
    }

    #[test]
    fn gap_documented_prime_degree_polynomials() {
        // GAP StandardFF docs:
        // StandardPrimeDegreePolynomial(13,3,1) = X^3 + Z(13)^7 = X^3 + 11.
        assert_eq!(
            standard_prime_degree_coefficients(13, 3, 1).unwrap(),
            vec![11, 0, 0]
        );
        // X^5 + Z(13)^4 X^2 + Z(13)^4 X - 1, with Z(13)=2.
        assert_eq!(
            standard_prime_degree_coefficients(13, 5, 1).unwrap(),
            vec![12, 3, 3, 0, 0]
        );
    }

    #[test]
    fn standard_gf4_relation_and_steinitz_rank() {
        let f = StandardFieldDescriptor::new(2, 2)
            .unwrap()
            .instantiate()
            .unwrap();
        let x = f.from_rank(2).unwrap();
        let x2 = f.mul(&x, &x).unwrap();
        // X^2 + X + 1 = 0 => X^2 = X + 1.
        assert_eq!(f.rank(&x2).unwrap(), 3);
    }

    #[test]
    fn embeddings_preserve_tower_basis_positions() {
        // First two tower monomials of degree 2 embed into degree 4 positions
        // selected by StdMonMap(2,4).
        assert_eq!(embed_steinitz(2, 2, 4, 2).unwrap(), 2);
    }

    #[test]
    fn every_small_standard_field_is_a_field() {
        for p in [2u32, 3, 5, 7] {
            for n in 1..=4 {
                let Some(q) = checked_pow(p as u128, n) else {
                    continue;
                };
                if q > 256 {
                    continue;
                }
                let f = StandardFieldDescriptor::new(p, n)
                    .unwrap()
                    .instantiate()
                    .unwrap();
                for rank in 1..f.cardinality() {
                    let x = f.from_rank(rank).unwrap();
                    let inv = f.inverse(&x).unwrap();
                    assert_eq!(f.mul(&x, &inv).unwrap(), f.one());
                }
            }
        }
    }
}
