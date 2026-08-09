use std::collections::BTreeMap;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AbstractCookNode {
    pub test_id: u32,
    pub residual_branches: BTreeMap<u32, Box<AbstractCookNode>>,
}
