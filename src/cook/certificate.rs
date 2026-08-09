use num_bigint::BigUint;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactCertificate {
    pub total_weight: BigUint,
    pub optimum_cost: BigUint,
    pub root_lower_bound: BigUint,
    pub explored_states: u64,
    pub memoized_states: u64,
}
