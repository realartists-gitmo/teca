use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplicitField {
    p: u32,
    degree: usize,
    cardinality: u128,
    model: FieldModel,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FieldModel {
    /// Simple polynomial basis over the prime field.
    Polynomial { modulus: Vec<u32> },
    /// StandardFF tower basis. Elements are flattened coefficient vectors over
    /// F_p; blocks of `base.degree` coefficients are coefficients of successive
    /// powers of the newly adjoined standard generator.
    Tower {
        base: Box<ExplicitField>,
        relative_degree: usize,
        /// Low coefficients of `X^r + sum modulus[i] X^i`, padded to r.
        modulus: Vec<FieldElement>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FieldElement {
    coeffs: Vec<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldError {
    InvalidCharacteristic(u32),
    InvalidModulus,
    CardinalityOverflow,
    RankOutOfRange {
        rank: u128,
        cardinality: u128,
    },
    WrongDegree {
        expected: usize,
        actual: usize,
    },
    CoefficientOutOfRange {
        coefficient: u32,
        characteristic: u32,
    },
    NonInvertible,
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCharacteristic(p) => write!(f, "invalid field characteristic {p}"),
            Self::InvalidModulus => write!(f, "invalid extension-field modulus"),
            Self::CardinalityOverflow => write!(f, "field cardinality does not fit u128"),
            Self::RankOutOfRange { rank, cardinality } => {
                write!(f, "field rank {rank} is outside cardinality {cardinality}")
            }
            Self::WrongDegree { expected, actual } => {
                write!(
                    f,
                    "field element has {actual} coefficients; expected {expected}"
                )
            }
            Self::CoefficientOutOfRange {
                coefficient,
                characteristic,
            } => write!(
                f,
                "field coefficient {coefficient} is outside F_{characteristic}"
            ),
            Self::NonInvertible => write!(f, "zero has no multiplicative inverse"),
        }
    }
}

impl std::error::Error for FieldError {}

impl ExplicitField {
    pub fn prime(p: u32) -> Result<Self, FieldError> {
        Self::new(p, vec![0, 1])
    }

    /// Construct a simple polynomial-basis field over F_p. This constructor is
    /// crate-private because public extension fields are standardized by
    /// `field::standardff`.
    pub(crate) fn new(p: u32, modulus: Vec<u32>) -> Result<Self, FieldError> {
        if !is_prime_u32(p) {
            return Err(FieldError::InvalidCharacteristic(p));
        }
        if modulus.len() < 2 || modulus.last().copied() != Some(1) {
            return Err(FieldError::InvalidModulus);
        }
        let degree = modulus.len() - 1;
        if modulus[..degree].iter().any(|&x| x >= p) {
            return Err(FieldError::InvalidModulus);
        }
        let cardinality =
            checked_pow_u128(p as u128, degree as u32).ok_or(FieldError::CardinalityOverflow)?;
        Ok(Self {
            p,
            degree,
            cardinality,
            model: FieldModel::Polynomial { modulus },
        })
    }

    /// Construct one StandardFF tower step `K[X]/f` where K is itself a
    /// standardized field and f is monic of `relative_degree`. Coefficients are
    /// supplied by their Steinitz numbers in K.
    pub(crate) fn tower(
        base: ExplicitField,
        relative_degree: usize,
        coefficient_ranks: &[u128],
    ) -> Result<Self, FieldError> {
        if relative_degree < 2 || coefficient_ranks.len() > relative_degree {
            return Err(FieldError::InvalidModulus);
        }
        let degree = base
            .degree
            .checked_mul(relative_degree)
            .ok_or(FieldError::CardinalityOverflow)?;
        let cardinality = checked_pow_u128(base.cardinality, relative_degree as u32)
            .ok_or(FieldError::CardinalityOverflow)?;
        let mut modulus = Vec::with_capacity(relative_degree);
        for i in 0..relative_degree {
            modulus.push(base.from_rank(*coefficient_ranks.get(i).unwrap_or(&0))?);
        }
        Ok(Self {
            p: base.p,
            degree,
            cardinality,
            model: FieldModel::Tower {
                base: Box::new(base),
                relative_degree,
                modulus,
            },
        })
    }

    pub const fn characteristic(&self) -> u32 {
        self.p
    }

    pub const fn degree(&self) -> usize {
        self.degree
    }

    pub const fn cardinality(&self) -> u128 {
        self.cardinality
    }

    pub fn zero(&self) -> FieldElement {
        FieldElement {
            coeffs: vec![0; self.degree],
        }
    }

    pub fn one(&self) -> FieldElement {
        let mut coeffs = vec![0; self.degree];
        coeffs[0] = 1 % self.p;
        FieldElement { coeffs }
    }

    pub fn from_prime(&self, value: u32) -> FieldElement {
        let mut coeffs = vec![0; self.degree];
        coeffs[0] = value % self.p;
        FieldElement { coeffs }
    }

    /// Steinitz number in the StandardFF tower basis: base-p digits, least
    /// significant/tower-basis coefficient first. For the simple polynomial model
    /// this is the historical polynomial-basis rank.
    pub fn from_rank(&self, mut rank: u128) -> Result<FieldElement, FieldError> {
        if rank >= self.cardinality {
            return Err(FieldError::RankOutOfRange {
                rank,
                cardinality: self.cardinality,
            });
        }
        let mut coeffs = vec![0; self.degree];
        for coeff in &mut coeffs {
            *coeff = (rank % self.p as u128) as u32;
            rank /= self.p as u128;
        }
        Ok(FieldElement { coeffs })
    }

    pub fn rank(&self, element: &FieldElement) -> Result<u128, FieldError> {
        self.check(element)?;
        let mut rank = 0u128;
        let mut place = 1u128;
        for (index, &coeff) in element.coeffs.iter().enumerate() {
            rank = rank
                .checked_add(
                    (coeff as u128)
                        .checked_mul(place)
                        .ok_or(FieldError::CardinalityOverflow)?,
                )
                .ok_or(FieldError::CardinalityOverflow)?;
            if index + 1 != element.coeffs.len() {
                place = place
                    .checked_mul(self.p as u128)
                    .ok_or(FieldError::CardinalityOverflow)?;
            }
        }
        Ok(rank)
    }

    pub fn add(&self, a: &FieldElement, b: &FieldElement) -> Result<FieldElement, FieldError> {
        self.check(a)?;
        self.check(b)?;
        Ok(FieldElement {
            coeffs: a
                .coeffs
                .iter()
                .zip(&b.coeffs)
                .map(|(&x, &y)| ((x as u64 + y as u64) % self.p as u64) as u32)
                .collect(),
        })
    }

    pub fn neg(&self, a: &FieldElement) -> Result<FieldElement, FieldError> {
        self.check(a)?;
        Ok(FieldElement {
            coeffs: a
                .coeffs
                .iter()
                .map(|&x| if x == 0 { 0 } else { self.p - x })
                .collect(),
        })
    }

    pub fn sub(&self, a: &FieldElement, b: &FieldElement) -> Result<FieldElement, FieldError> {
        self.check(a)?;
        self.check(b)?;
        Ok(FieldElement {
            coeffs: a
                .coeffs
                .iter()
                .zip(&b.coeffs)
                .map(|(&x, &y)| ((x as u64 + self.p as u64 - y as u64) % self.p as u64) as u32)
                .collect(),
        })
    }

    pub fn mul(&self, a: &FieldElement, b: &FieldElement) -> Result<FieldElement, FieldError> {
        self.check(a)?;
        self.check(b)?;
        match &self.model {
            FieldModel::Polynomial { modulus } => self.mul_polynomial(a, b, modulus),
            FieldModel::Tower {
                base,
                relative_degree,
                modulus,
            } => self.mul_tower(a, b, base, *relative_degree, modulus),
        }
    }

    fn mul_polynomial(
        &self,
        a: &FieldElement,
        b: &FieldElement,
        modulus: &[u32],
    ) -> Result<FieldElement, FieldError> {
        let mut tmp = vec![0u128; self.degree * 2 - 1];
        let p = self.p as u128;
        for (i, &x) in a.coeffs.iter().enumerate() {
            for (j, &y) in b.coeffs.iter().enumerate() {
                tmp[i + j] = (tmp[i + j] + x as u128 * y as u128) % p;
            }
        }

        // x^degree = -sum modulus[i] x^i.
        for power in (self.degree..tmp.len()).rev() {
            let factor = tmp[power] % p;
            if factor == 0 {
                continue;
            }
            tmp[power] = 0;
            let shift = power - self.degree;
            for i in 0..self.degree {
                let subtract = factor * modulus[i] as u128 % p;
                tmp[shift + i] = (tmp[shift + i] + p - subtract) % p;
            }
        }

        Ok(FieldElement {
            coeffs: tmp[..self.degree].iter().map(|&x| x as u32).collect(),
        })
    }

    fn mul_tower(
        &self,
        a: &FieldElement,
        b: &FieldElement,
        base: &ExplicitField,
        relative_degree: usize,
        modulus: &[FieldElement],
    ) -> Result<FieldElement, FieldError> {
        let block = base.degree;
        let split = |value: &FieldElement| -> Vec<FieldElement> {
            value
                .coeffs
                .chunks_exact(block)
                .map(|chunk| FieldElement {
                    coeffs: chunk.to_vec(),
                })
                .collect()
        };
        let aa = split(a);
        let bb = split(b);
        let mut tmp = vec![base.zero(); relative_degree * 2 - 1];
        for (i, x) in aa.iter().enumerate() {
            for (j, y) in bb.iter().enumerate() {
                let product = base.mul(x, y)?;
                tmp[i + j] = base.add(&tmp[i + j], &product)?;
            }
        }
        for power in (relative_degree..tmp.len()).rev() {
            let factor = tmp[power].clone();
            if factor.is_zero() {
                continue;
            }
            tmp[power] = base.zero();
            let shift = power - relative_degree;
            for i in 0..relative_degree {
                let product = base.mul(&factor, &modulus[i])?;
                tmp[shift + i] = base.sub(&tmp[shift + i], &product)?;
            }
        }
        let mut coeffs = Vec::with_capacity(self.degree);
        for coefficient in tmp.into_iter().take(relative_degree) {
            coeffs.extend_from_slice(coefficient.coefficients());
        }
        Ok(FieldElement { coeffs })
    }

    pub fn pow(&self, base: &FieldElement, mut exponent: u128) -> Result<FieldElement, FieldError> {
        self.check(base)?;
        let mut out = self.one();
        let mut cur = base.clone();
        while exponent != 0 {
            if exponent & 1 != 0 {
                out = self.mul(&out, &cur)?;
            }
            exponent >>= 1;
            if exponent != 0 {
                cur = self.mul(&cur, &cur)?;
            }
        }
        Ok(out)
    }

    pub fn inverse(&self, value: &FieldElement) -> Result<FieldElement, FieldError> {
        self.check(value)?;
        if value.is_zero() {
            return Err(FieldError::NonInvertible);
        }
        self.pow(value, self.cardinality - 2)
    }

    fn check(&self, value: &FieldElement) -> Result<(), FieldError> {
        if value.coeffs.len() != self.degree {
            return Err(FieldError::WrongDegree {
                expected: self.degree,
                actual: value.coeffs.len(),
            });
        }
        if let Some(&coefficient) = value.coeffs.iter().find(|&&x| x >= self.p) {
            return Err(FieldError::CoefficientOutOfRange {
                coefficient,
                characteristic: self.p,
            });
        }
        Ok(())
    }
}

impl FieldElement {
    pub fn coefficients(&self) -> &[u32] {
        &self.coeffs
    }

    pub fn is_zero(&self) -> bool {
        self.coeffs.iter().all(|&x| x == 0)
    }
}

fn checked_pow_u128(mut base: u128, mut exponent: u32) -> Option<u128> {
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
    fn prime_field_inverse() {
        let f = ExplicitField::prime(107).unwrap();
        let x = f.from_rank(17).unwrap();
        let inv = f.inverse(&x).unwrap();
        assert_eq!(f.rank(&f.mul(&x, &inv).unwrap()).unwrap(), 1);
    }

    #[test]
    fn tower_quadratic_relation() {
        let base = ExplicitField::prime(3).unwrap();
        // X^2 + 1 over F3.
        let f = ExplicitField::tower(base, 2, &[1]).unwrap();
        let x = f.from_rank(3).unwrap();
        let x2 = f.mul(&x, &x).unwrap();
        assert_eq!(f.rank(&x2).unwrap(), 2);
    }
}
