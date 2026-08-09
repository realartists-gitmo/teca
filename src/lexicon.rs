use std::collections::BTreeSet;
use std::fmt;

use crate::address::AtomId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lexicon {
    atoms: Vec<Vec<u8>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LexiconError {
    TooSmall(usize),
    TooLarge(usize),
    DuplicateAtom(Vec<u8>),
    AtomOutOfRange { atom: u32, capacity: usize },
}

impl fmt::Display for LexiconError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooSmall(n) => write!(f, "TECA lexicon needs at least 2 atoms, got {n}"),
            Self::TooLarge(n) => write!(f, "TECA lexicon capacity must fit u32, got {n}"),
            Self::DuplicateAtom(atom) => write!(f, "duplicate lexicon atom: {atom:?}"),
            Self::AtomOutOfRange { atom, capacity } => {
                write!(f, "atom {atom} is outside lexicon capacity {capacity}")
            }
        }
    }
}

impl std::error::Error for LexiconError {}

impl Lexicon {
    pub fn new(atoms: Vec<Vec<u8>>) -> Result<Self, LexiconError> {
        if atoms.len() < 2 {
            return Err(LexiconError::TooSmall(atoms.len()));
        }
        if atoms.len() > u32::MAX as usize {
            return Err(LexiconError::TooLarge(atoms.len()));
        }
        let mut seen = BTreeSet::new();
        for atom in &atoms {
            if !seen.insert(atom.clone()) {
                return Err(LexiconError::DuplicateAtom(atom.clone()));
            }
        }
        Ok(Self { atoms })
    }

    pub fn capacity(&self) -> usize {
        self.atoms.len()
    }

    pub fn atom(&self, id: AtomId) -> Result<&[u8], LexiconError> {
        self.atoms
            .get(id.0 as usize)
            .map(Vec::as_slice)
            .ok_or(LexiconError::AtomOutOfRange {
                atom: id.0,
                capacity: self.atoms.len(),
            })
    }

    pub fn atoms(&self) -> impl ExactSizeIterator<Item = &[u8]> {
        self.atoms.iter().map(Vec::as_slice)
    }
}
