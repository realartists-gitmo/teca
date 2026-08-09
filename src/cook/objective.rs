use num_bigint::BigUint;
use num_traits::Zero;

/// Exact weighted expected-depth numerator for already-known distinguishing
/// depths. The caller owns any common denominator/normalization.
pub fn weighted_depth_cost(weights: &[BigUint], depths: &[u32]) -> Option<BigUint> {
    if weights.len() != depths.len() {
        return None;
    }
    let mut out = BigUint::zero();
    for (w, &d) in weights.iter().zip(depths) {
        out += w * BigUint::from(d);
    }
    Some(out)
}
