use super::hasse::binomial_mod_prime;
use super::{ExplicitField, FieldElement, FieldError};

/// Evaluate the Hasse derivative of a polynomial whose coefficients live in the
/// prime subfield. Coefficients are low degree first.
pub fn evaluate_hasse(
    field: &ExplicitField,
    prime_coefficients: &[u32],
    order: usize,
    point: &FieldElement,
) -> Result<FieldElement, FieldError> {
    if order >= prime_coefficients.len() {
        return Ok(field.zero());
    }

    let mut out = field.zero();
    let mut power = field.one();
    for (i, &coefficient) in prime_coefficients.iter().enumerate().skip(order) {
        let choose = binomial_mod_prime(i, order, field.characteristic());
        let scalar = ((coefficient as u64 * choose as u64) % field.characteristic() as u64) as u32;
        if scalar != 0 {
            let term = field.mul(&field.from_prime(scalar), &power)?;
            out = field.add(&out, &term)?;
        }
        power = field.mul(&power, point)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hasse_at_zero_returns_coefficient() {
        let f = ExplicitField::prime(11).unwrap();
        let zero = f.zero();
        let p = [3, 5, 7, 1];
        for (r, &expected) in p.iter().enumerate() {
            let got = evaluate_hasse(&f, &p, r, &zero).unwrap();
            assert_eq!(f.rank(&got).unwrap(), expected as u128);
        }
    }
}
