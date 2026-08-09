use num_bigint::BigUint;

/// First-step lower bound for a state with total weight `W` and the minimum
/// residual collision weight `R` achievable by any legal next action.
///
/// Every scenario pays one atom now; every residual scenario must pay at least
/// one additional atom later, hence OPT >= W + R.
pub fn first_step_lower_bound(total_weight: &BigUint, min_residual: &BigUint) -> BigUint {
    total_weight + min_residual
}
