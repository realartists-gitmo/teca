use std::fmt;

use crate::address::{Address, AtomId};
use crate::bundle::{BundleError, ProbeBundle};
use crate::codec::ContentCodecDescriptor;
use crate::fallback::{FallbackDescriptor, FallbackError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionNode {
    pub action: ProbeBundle,
    /// Sparse branch overrides sorted by AtomId. `None` means fallback.
    pub branches: Vec<(AtomId, u32)>,
    /// Optional default child for all outcomes without an explicit override.
    pub default_child: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Scheme {
    pub capacity: u32,
    pub codec: ContentCodecDescriptor,
    pub root: Option<u32>,
    pub nodes: Vec<DecisionNode>,
    pub fallback: FallbackDescriptor,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchemeError {
    CapacityTooSmall(u32),
    InvalidRoot(u32),
    InvalidNode(u32),
    InvalidBranchOrder,
    BranchAtomOutOfRange { atom: u32, capacity: u32 },
    BranchAtomOutOfActionRange { atom: u32, outcomes: u128 },
    RootMustBeZero(u32),
    OrphanNodesWithoutRoot,
    UnreachableNode(u32),
    BackEdge { from: u32, to: u32 },
    Bundle(BundleError),
    Fallback(FallbackError),
}

impl fmt::Display for SchemeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityTooSmall(n) => write!(f, "scheme capacity must be >= 2, got {n}"),
            Self::InvalidRoot(id) => write!(f, "invalid root node {id}"),
            Self::InvalidNode(id) => write!(f, "invalid decision node {id}"),
            Self::InvalidBranchOrder => {
                write!(f, "decision-node branch atoms must be strictly sorted")
            }
            Self::BranchAtomOutOfRange { atom, capacity } => {
                write!(f, "branch atom {atom} is outside capacity {capacity}")
            }
            Self::BranchAtomOutOfActionRange { atom, outcomes } => {
                write!(
                    f,
                    "branch atom {atom} is outside action outcome cardinality {outcomes}"
                )
            }
            Self::RootMustBeZero(root) => write!(
                f,
                "canonical decision graph root must be node 0, got {root}"
            ),
            Self::OrphanNodesWithoutRoot => {
                write!(f, "scheme without a root must not contain decision nodes")
            }
            Self::UnreachableNode(node) => {
                write!(f, "decision node {node} is unreachable from root")
            }
            Self::BackEdge { from, to } => {
                write!(f, "decision graph must be acyclic/forward: {from}->{to}")
            }
            Self::Bundle(err) => err.fmt(f),
            Self::Fallback(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for SchemeError {}

impl From<BundleError> for SchemeError {
    fn from(value: BundleError) -> Self {
        Self::Bundle(value)
    }
}
impl From<FallbackError> for SchemeError {
    fn from(value: FallbackError) -> Self {
        Self::Fallback(value)
    }
}

impl DecisionNode {
    pub fn child_for(&self, atom: AtomId) -> Option<u32> {
        match self.branches.binary_search_by_key(&atom, |(key, _)| *key) {
            Ok(index) => Some(self.branches[index].1),
            Err(_) => self.default_child,
        }
    }
}

impl Scheme {
    pub fn fallback_only(
        capacity: u32,
        codec: ContentCodecDescriptor,
    ) -> Result<Self, SchemeError> {
        let scheme = Self {
            capacity,
            codec,
            root: None,
            nodes: Vec::new(),
            fallback: FallbackDescriptor::DirectRadixV1,
        };
        scheme.validate()?;
        Ok(scheme)
    }

    pub fn validate(&self) -> Result<(), SchemeError> {
        if self.capacity < 2 {
            return Err(SchemeError::CapacityTooSmall(self.capacity));
        }
        self.fallback.validate(self.capacity)?;
        match self.root {
            None if !self.nodes.is_empty() => return Err(SchemeError::OrphanNodesWithoutRoot),
            Some(root) if root as usize >= self.nodes.len() => {
                return Err(SchemeError::InvalidRoot(root));
            }
            Some(root) if root != 0 => return Err(SchemeError::RootMustBeZero(root)),
            _ => {}
        }
        let mut reachable = vec![false; self.nodes.len()];
        if self.root.is_some() {
            reachable[0] = true;
        }
        for (from, node) in self.nodes.iter().enumerate() {
            let outcomes = node.action.validate(self.capacity)?;
            let mut prev = None;
            for &(atom, child) in &node.branches {
                if atom.0 >= self.capacity {
                    return Err(SchemeError::BranchAtomOutOfRange {
                        atom: atom.0,
                        capacity: self.capacity,
                    });
                }
                if atom.0 as u128 >= outcomes {
                    return Err(SchemeError::BranchAtomOutOfActionRange {
                        atom: atom.0,
                        outcomes,
                    });
                }
                if prev.is_some_and(|p| atom.0 <= p) {
                    return Err(SchemeError::InvalidBranchOrder);
                }
                prev = Some(atom.0);
                if child as usize >= self.nodes.len() {
                    return Err(SchemeError::InvalidNode(child));
                }
                if child <= from as u32 {
                    return Err(SchemeError::BackEdge {
                        from: from as u32,
                        to: child,
                    });
                }
                if reachable[from] {
                    reachable[child as usize] = true;
                }
            }
            if let Some(child) = node.default_child {
                if child as usize >= self.nodes.len() {
                    return Err(SchemeError::InvalidNode(child));
                }
                if child <= from as u32 {
                    return Err(SchemeError::BackEdge {
                        from: from as u32,
                        to: child,
                    });
                }
                if reachable[from] {
                    reachable[child as usize] = true;
                }
            }
        }
        if let Some((node, _)) = reachable.iter().enumerate().find(|(_, seen)| !**seen) {
            return Err(SchemeError::UnreachableNode(node as u32));
        }
        Ok(())
    }

    pub fn address<'a>(&'a self, bytes: &'a [u8]) -> Address<'a> {
        Address::new(self, bytes)
    }

    pub fn prefix(&self, bytes: &[u8], depth: usize) -> Vec<AtomId> {
        self.address(bytes).take(depth).collect()
    }
}
