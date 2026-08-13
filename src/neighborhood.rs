//! Canonical stateful neighborhood addressing layer.
//!
//! The low-level [`Scheme`] maps caller bytes to an indefinitely extensible
//! structural [`AtomId`] stream. `Neighborhood` owns the canonical rules for
//! turning colliding streams into shortest-unique addresses over a changing set
//! of co-occurring contents. This is TECA semantics, not a database abstraction.
//!
//! Core invariant: for each distinct byte string `b` the current canonical
//! neighborhood address is the shortest nonempty prefix of `S(b)` that no other
//! distinct member's stream shares. For a one-member neighborhood the address
//! is exactly one atom.
//!
//! Addresses depend only on the scheme and the current set of distinct byte
//! strings, never on insertion order. Inserting a colliding member may lengthen
//! existing addresses; removing a member may shorten remaining addresses.
//! Identifiers are opaque downstream bytes and never affect address generation.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::address::AtomId;
use crate::defaults::default_scheme;
use crate::scheme::{Scheme, SchemeError};

/// Sentinel stored inside fresh content marker nodes before the entry is
/// actually committed to the entries vector.
const PLACEHOLDER: usize = usize::MAX;

/// Safety bound on same-stream comparison. A validated scheme should diverge
/// for distinct byte strings within the informative stream; this guards against
/// pathological hand-built schemes that would otherwise loop forever.
const MAX_DIVERGENCE_ATOMS: usize = 65_536;

/// A structural neighborhood address: a nonempty prefix of a TECA stream.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NeighborhoodAddress {
    atoms: Vec<AtomId>,
}

impl NeighborhoodAddress {
    pub fn new(atoms: Vec<AtomId>) -> Result<Self, NeighborhoodError> {
        if atoms.is_empty() {
            return Err(NeighborhoodError::EmptyAddress);
        }
        Ok(Self { atoms })
    }

    pub fn atoms(&self) -> &[AtomId] {
        &self.atoms
    }

    pub fn len(&self) -> usize {
        self.atoms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.atoms.is_empty()
    }
}

/// One row of the canonical neighborhood relation: TECA address, exact source
/// bytes, and an optional opaque downstream identifier.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NeighborhoodEntry {
    address: NeighborhoodAddress,
    bytes: Vec<u8>,
    identifier: Option<Vec<u8>>,
}

impl NeighborhoodEntry {
    pub fn address(&self) -> &NeighborhoodAddress {
        &self.address
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn identifier(&self) -> Option<&[u8]> {
        self.identifier.as_deref()
    }
}

/// Derived from the low-level scheme and lexicon layers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NeighborhoodError {
    InvalidScheme(SchemeError),
    EmptyAddress,
    AtomOutOfCapacity { atom: u32, capacity: u32 },
    IdentifierAlreadyAssigned(Vec<u8>),
    ConflictingIdentifierForContent(Vec<u8>),
    ContentNotFound,
    SchemeMismatch,
    ContentsNotDistinguishable { left: Vec<u8>, right: Vec<u8> },
    InvalidState(&'static str),
}

impl fmt::Display for NeighborhoodError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScheme(err) => err.fmt(f),
            Self::EmptyAddress => write!(f, "neighborhood addresses must be nonempty"),
            Self::AtomOutOfCapacity { atom, capacity } => {
                write!(
                    f,
                    "address atom {atom} is outside scheme capacity {capacity}"
                )
            }
            Self::IdentifierAlreadyAssigned(id) => {
                write!(
                    f,
                    "identifier {:?} is already assigned to another content",
                    id
                )
            }
            Self::ConflictingIdentifierForContent(id) => {
                write!(
                    f,
                    "existing content already carries a different identifier {:?}",
                    id
                )
            }
            Self::ContentNotFound => write!(f, "content is not present in the neighborhood"),
            Self::SchemeMismatch => write!(f, "neighborhoods must share an equal scheme to merge"),
            Self::ContentsNotDistinguishable { left, right } => write!(
                f,
                "scheme does not distinguish distinct contents {:?} and {:?} within the probing bound",
                left, right
            ),
            Self::InvalidState(msg) => write!(f, "invalid neighborhood state: {msg}"),
        }
    }
}

impl std::error::Error for NeighborhoodError {}

impl From<SchemeError> for NeighborhoodError {
    fn from(value: SchemeError) -> Self {
        Self::InvalidScheme(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertStatus {
    Inserted,
    Existing,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsertResult {
    pub status: InsertStatus,
    pub address: NeighborhoodAddress,
    pub changes: Vec<AddressChange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddressChange {
    pub bytes: Vec<u8>,
    pub identifier: Option<Vec<u8>>,
    pub old: NeighborhoodAddress,
    pub new: NeighborhoodAddress,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoveResult {
    pub removed: NeighborhoodEntry,
    pub changes: Vec<AddressChange>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Resolution<'a> {
    NotFound,
    Unique(&'a NeighborhoodEntry),
    Ambiguous { matches: usize },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NeighborhoodRow {
    pub bytes: Vec<u8>,
    pub identifier: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
struct TrieNode {
    children: BTreeMap<AtomId, u32>,
    content: Option<usize>,
    count: usize,
}

impl TrieNode {
    fn empty() -> Self {
        Self {
            children: BTreeMap::new(),
            content: None,
            count: 0,
        }
    }
}

/// A read-only description of where and how a new distinct content attaches to
/// the trie. Built before any mutation so every error path leaves the
/// neighborhood untouched.
enum InsertPlan {
    NewLeaf {
        parent: u32,
        atom: AtomId,
        ancestors: Vec<u32>,
        prefix_atoms: Vec<AtomId>,
    },
    SplitContent {
        base: u32,
        member: usize,
        ancestors: Vec<u32>,
        prefix_atoms: Vec<AtomId>,
        shared: Vec<AtomId>,
        b_atom: AtomId,
        m_atom: AtomId,
    },
}

/// A stateful namespace layer that owns canonical shortest-unique address
/// assignment over a set of co-occurring contents.
#[derive(Clone, Debug)]
pub struct Neighborhood {
    scheme: Scheme,
    /// Authoritative rows, always sorted by lexicographic input-bytes order.
    entries: Vec<NeighborhoodEntry>,
    /// Deterministic prefix trie over materialized `AtomId` divisors. Index 0
    /// is the root. `children` is ordered by `AtomId`.
    trie: Vec<TrieNode>,
    /// Recycled trie-node slots, parallel to nothing. Freed nodes are never
    /// reachable from the root.
    free_nodes: Vec<u32>,
    /// For each entry index, the trie node holding its content marker.
    entry_nodes: Vec<u32>,
    /// Opaque identifier -> entry index.
    identifier_index: BTreeMap<Vec<u8>, usize>,
}

impl Neighborhood {
    pub fn new(scheme: Scheme) -> Result<Self, NeighborhoodError> {
        scheme.validate()?;
        Ok(Self {
            scheme,
            entries: Vec::new(),
            trie: vec![TrieNode::empty()],
            free_nodes: Vec::new(),
            entry_nodes: Vec::new(),
            identifier_index: BTreeMap::new(),
        })
    }

    /// An empty neighborhood using the canonical embedded default scheme.
    pub fn canonical() -> Self {
        Self::new(default_scheme().clone()).expect("embedded canonical scheme must validate")
    }

    pub fn scheme(&self) -> &Scheme {
        &self.scheme
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Deterministic iteration in lexicographic input-byte order.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &NeighborhoodEntry> {
        self.entries.iter()
    }

    pub fn insert(&mut self, bytes: impl Into<Vec<u8>>) -> Result<InsertResult, NeighborhoodError> {
        self.insert_inner(bytes.into(), None)
    }

    pub fn insert_with_identifier(
        &mut self,
        bytes: impl Into<Vec<u8>>,
        identifier: impl Into<Vec<u8>>,
    ) -> Result<InsertResult, NeighborhoodError> {
        self.insert_inner(bytes.into(), Some(identifier.into()))
    }

    pub fn set_identifier(
        &mut self,
        bytes: &[u8],
        identifier: Option<Vec<u8>>,
    ) -> Result<(), NeighborhoodError> {
        let pos = self
            .entries
            .binary_search_by(|e| e.bytes.as_slice().cmp(bytes))
            .map_err(|_| NeighborhoodError::ContentNotFound)?;
        if let Some(id) = &identifier {
            if let Some(&other) = self.identifier_index.get(id)
                && other != pos
            {
                return Err(NeighborhoodError::IdentifierAlreadyAssigned(id.clone()));
            }
        } else if let Some(id) = self.entries[pos].identifier.take() {
            self.identifier_index.remove(&id);
        }
        if let Some(id) = &identifier {
            self.identifier_index.insert(id.clone(), pos);
            self.entries[pos].identifier = Some(id.clone());
        }
        Ok(())
    }

    pub fn get_by_bytes(&self, bytes: &[u8]) -> Option<&NeighborhoodEntry> {
        self.entries
            .binary_search_by(|e| e.bytes.as_slice().cmp(bytes))
            .ok()
            .map(|i| &self.entries[i])
    }

    pub fn get_by_identifier(&self, identifier: &[u8]) -> Option<&NeighborhoodEntry> {
        self.identifier_index
            .get(identifier)
            .map(|&i| &self.entries[i])
    }

    /// Address lookup is prefix resolution against the current neighborhoods,
    /// not mere equality against the currently minted shortest address.
    pub fn resolve(
        &self,
        address: &NeighborhoodAddress,
    ) -> Result<Resolution<'_>, NeighborhoodError> {
        if address.is_empty() {
            return Err(NeighborhoodError::EmptyAddress);
        }
        for a in &address.atoms {
            if a.0 >= self.scheme.capacity {
                return Err(NeighborhoodError::AtomOutOfCapacity {
                    atom: a.0,
                    capacity: self.scheme.capacity,
                });
            }
        }
        let atoms = &address.atoms;
        let mut node = 0u32;
        for i in 0..atoms.len() {
            if let Some(content) = self.trie[node as usize].content {
                // The query extends past this member's canonical address; keep
                // checking against the actual continued TECA stream.
                let mut stream = self.scheme.address(&self.entries[content].bytes).skip(i);
                if atoms[i..].iter().all(|&want| stream.next() == Some(want)) {
                    return Ok(Resolution::Unique(&self.entries[content]));
                }
                return Ok(Resolution::NotFound);
            }
            match self.trie[node as usize].children.get(&atoms[i]) {
                Some(&child) => node = child,
                None => return Ok(Resolution::NotFound),
            }
        }
        if let Some(content) = self.trie[node as usize].content {
            return Ok(Resolution::Unique(&self.entries[content]));
        }
        match self.trie[node as usize].count {
            0 => Ok(Resolution::NotFound),
            1 => {
                let member = self.sole_member_at(node).unwrap();
                Ok(Resolution::Unique(&self.entries[member]))
            }
            n => Ok(Resolution::Ambiguous { matches: n }),
        }
    }

    pub fn remove_by_bytes(
        &mut self,
        bytes: &[u8],
    ) -> Result<Option<RemoveResult>, NeighborhoodError> {
        let pos = match self
            .entries
            .binary_search_by(|e| e.bytes.as_slice().cmp(bytes))
        {
            Ok(pos) => pos,
            Err(_) => return Ok(None),
        };
        Ok(Some(self.remove_inner(pos)))
    }

    pub fn remove_by_identifier(
        &mut self,
        identifier: &[u8],
    ) -> Result<Option<RemoveResult>, NeighborhoodError> {
        let Some(&pos) = self.identifier_index.get(identifier) else {
            return Ok(None);
        };
        Ok(Some(self.remove_inner(pos)))
    }

    pub fn from_rows(
        scheme: Scheme,
        rows: impl IntoIterator<Item = NeighborhoodRow>,
    ) -> Result<Neighborhood, NeighborhoodError> {
        let mut neighborhood = Neighborhood::new(scheme)?;
        for row in rows {
            match row.identifier {
                None => {
                    neighborhood.insert(row.bytes)?;
                }
                Some(identifier) => {
                    neighborhood.insert_with_identifier(row.bytes, identifier)?;
                }
            }
        }
        Ok(neighborhood)
    }

    /// Merge `other` into `self`, equivalent to constructing a fresh
    /// neighborhood over the union of contents. The schemes must be equal.
    /// Transactional: on `Err`, `self` is unchanged.
    pub fn merge(&mut self, other: &Neighborhood) -> Result<Vec<AddressChange>, NeighborhoodError> {
        if self.scheme != other.scheme {
            return Err(NeighborhoodError::SchemeMismatch);
        }
        let mut working = self.clone();
        for entry in &other.entries {
            match &entry.identifier {
                Some(id) => {
                    working.insert_with_identifier(entry.bytes.clone(), id.clone())?;
                }
                None => {
                    working.insert(entry.bytes.clone())?;
                }
            }
        }
        let mut changes = Vec::new();
        for old in &self.entries {
            let new = &working
                .entries
                .binary_search_by(|e| e.bytes.as_slice().cmp(&old.bytes))
                .map(|i| &working.entries[i])
                .expect("merge union must retain all prior contents");
            if new.address != old.address {
                changes.push(AddressChange {
                    bytes: old.bytes.clone(),
                    identifier: old.identifier.clone(),
                    old: old.address.clone(),
                    new: new.address.clone(),
                });
            }
        }
        changes.sort_by(|a, b| a.bytes.cmp(&b.bytes));
        *self = working;
        Ok(changes)
    }

    pub fn validate(&self) -> Result<(), NeighborhoodError> {
        self.scheme.validate()?;
        if self
            .entries
            .windows(2)
            .any(|w| w[0].bytes.as_slice() >= w[1].bytes.as_slice())
        {
            return Err(NeighborhoodError::InvalidState(
                "entries are not in strict canonical byte order",
            ));
        }
        let mut seen = BTreeSet::new();
        for entry in &self.entries {
            if let Some(id) = &entry.identifier
                && !seen.insert(id.clone())
            {
                return Err(NeighborhoodError::InvalidState(
                    "identifier assigned to more than one content",
                ));
            }
        }
        for entry in &self.entries {
            if entry.address.is_empty() {
                return Err(NeighborhoodError::EmptyAddress);
            }
            if let Some(&atom) = entry
                .address
                .atoms
                .iter()
                .find(|a| a.0 >= self.scheme.capacity)
            {
                return Err(NeighborhoodError::AtomOutOfCapacity {
                    atom: atom.0,
                    capacity: self.scheme.capacity,
                });
            }
            let mut stream = self.scheme.address(&entry.bytes);
            if !entry
                .address
                .atoms
                .iter()
                .all(|&want| stream.next() == Some(want))
            {
                return Err(NeighborhoodError::InvalidState(
                    "stored address is not a prefix of its content's stream",
                ));
            }
        }

        let rows = self
            .entries
            .iter()
            .map(|e| NeighborhoodRow {
                bytes: e.bytes.clone(),
                identifier: e.identifier.clone(),
            })
            .collect::<Vec<_>>();
        let reference = Neighborhood::from_rows(self.scheme.clone(), rows)
            .map_err(|_| NeighborhoodError::InvalidState("reference reconstruction failed"))?;
        if self.entries.len() != reference.entries.len() {
            return Err(NeighborhoodError::InvalidState(
                "reference reconstruction disagrees on entry count",
            ));
        }
        for (mine, theirs) in self.entries.iter().zip(reference.entries.iter()) {
            if mine.address != theirs.address {
                return Err(NeighborhoodError::InvalidState(
                    "stored address is not the canonical shortest unique address",
                ));
            }
        }
        if self.identifier_index != reference.identifier_index {
            return Err(NeighborhoodError::InvalidState(
                "identifier index disagrees with entries",
            ));
        }
        let mine = canonical_trie_form(&self.trie);
        let theirs = canonical_trie_form(&reference.trie);
        if mine != theirs {
            return Err(NeighborhoodError::InvalidState(
                "derived trie state disagrees with entries",
            ));
        }
        Ok(())
    }

    /// Rebuild all derived state and canonical addresses from the
    /// authoritative `(bytes, identifier)` rows. Returns the address changes.
    /// Transactional on error.
    pub fn rebuild(&mut self) -> Result<Vec<AddressChange>, NeighborhoodError> {
        let old: Vec<(Vec<u8>, Option<Vec<u8>>, NeighborhoodAddress)> = self
            .entries
            .iter()
            .map(|e| (e.bytes.clone(), e.identifier.clone(), e.address.clone()))
            .collect();
        let rows = self
            .entries
            .iter()
            .map(|e| NeighborhoodRow {
                bytes: e.bytes.clone(),
                identifier: e.identifier.clone(),
            })
            .collect::<Vec<_>>();
        let rebuilt = Neighborhood::from_rows(self.scheme.clone(), rows)?;
        let mut changes = Vec::new();
        for (bytes, identifier, old_address) in &old {
            let new_address = &rebuilt
                .entries
                .binary_search_by(|e| e.bytes.as_slice().cmp(bytes))
                .map(|i| &rebuilt.entries[i])
                .expect("rows must survive a rebuild")
                .address;
            if new_address != old_address {
                changes.push(AddressChange {
                    bytes: bytes.clone(),
                    identifier: identifier.clone(),
                    old: old_address.clone(),
                    new: new_address.clone(),
                });
            }
        }
        changes.sort_by(|a, b| a.bytes.cmp(&b.bytes));
        *self = rebuilt;
        Ok(changes)
    }

    fn insert_inner(
        &mut self,
        bytes: Vec<u8>,
        identifier: Option<Vec<u8>>,
    ) -> Result<InsertResult, NeighborhoodError> {
        match self
            .entries
            .binary_search_by(|e| e.bytes.as_slice().cmp(&bytes))
        {
            Ok(pos) => {
                let address = self.entries[pos].address.clone();
                let existing = self.entries[pos].identifier.as_deref();
                match (existing, identifier) {
                    (_, None) => Ok(InsertResult {
                        status: InsertStatus::Existing,
                        address,
                        changes: Vec::new(),
                    }),
                    (Some(existing), Some(wanted)) if existing == wanted => Ok(InsertResult {
                        status: InsertStatus::Existing,
                        address,
                        changes: Vec::new(),
                    }),
                    (Some(_), Some(wanted)) => {
                        Err(NeighborhoodError::ConflictingIdentifierForContent(wanted))
                    }
                    (None, Some(wanted)) => {
                        if self.identifier_index.contains_key(&wanted) {
                            return Err(NeighborhoodError::IdentifierAlreadyAssigned(wanted));
                        }
                        self.entries[pos].identifier = Some(wanted);
                        self.identifier_index
                            .insert(self.entries[pos].identifier.clone().unwrap(), pos);
                        Ok(InsertResult {
                            status: InsertStatus::Existing,
                            address,
                            changes: Vec::new(),
                        })
                    }
                }
            }
            Err(pos) => {
                if let Some(id) = &identifier
                    && self.identifier_index.contains_key(id)
                {
                    return Err(NeighborhoodError::IdentifierAlreadyAssigned(id.clone()));
                }
                let plan = self.plan_insert(&bytes)?;
                Ok(self.apply_insert(plan, pos, bytes, identifier))
            }
        }
    }

    /// Read-only trie walk deciding exactly how a new distinct content attaches.
    fn plan_insert(&self, bytes: &[u8]) -> Result<InsertPlan, NeighborhoodError> {
        let mut node = 0u32;
        let mut ancestors = vec![0u32];
        let mut prefix_atoms = Vec::<AtomId>::new();
        loop {
            if let Some(content) = self.trie[node as usize].content {
                let member_bytes = &self.entries[content].bytes;
                let base = prefix_atoms.len();
                let mut b_stream = self.scheme.address(bytes).skip(base);
                let mut m_stream = self.scheme.address(member_bytes).skip(base);
                let mut shared = Vec::new();
                for _ in 0..MAX_DIVERGENCE_ATOMS {
                    let b_atom = b_stream.next().expect("scheme streams are infinite");
                    let m_atom = m_stream.next().expect("scheme streams are infinite");
                    if b_atom == m_atom {
                        shared.push(b_atom);
                    } else {
                        return Ok(InsertPlan::SplitContent {
                            base: node,
                            member: content,
                            ancestors,
                            prefix_atoms,
                            shared,
                            b_atom,
                            m_atom,
                        });
                    }
                }
                return Err(NeighborhoodError::ContentsNotDistinguishable {
                    left: bytes.to_vec(),
                    right: member_bytes.clone(),
                });
            }
            let atom = self
                .scheme
                .address(bytes)
                .nth(prefix_atoms.len())
                .expect("scheme streams are infinite");
            match self.trie[node as usize].children.get(&atom) {
                Some(&child) => {
                    ancestors.push(child);
                    prefix_atoms.push(atom);
                    node = child;
                }
                None => {
                    return Ok(InsertPlan::NewLeaf {
                        parent: node,
                        atom,
                        ancestors,
                        prefix_atoms,
                    });
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn apply_insert(
        &mut self,
        plan: InsertPlan,
        pos: usize,
        bytes: Vec<u8>,
        identifier: Option<Vec<u8>>,
    ) -> InsertResult {
        let (mut changes, new_address, b_marker) = match plan {
            InsertPlan::NewLeaf {
                parent,
                atom,
                ancestors,
                prefix_atoms,
            } => {
                let b_marker = self.new_node();
                self.trie[b_marker as usize].content = Some(PLACEHOLDER);
                self.trie[b_marker as usize].count = 1;
                self.trie[parent as usize].children.insert(atom, b_marker);
                for &ancestor in &ancestors {
                    self.trie[ancestor as usize].count += 1;
                }
                let mut atoms = prefix_atoms;
                atoms.push(atom);
                (
                    Vec::new(),
                    NeighborhoodAddress::new(atoms).expect("nonempty"),
                    b_marker,
                )
            }
            InsertPlan::SplitContent {
                base,
                member,
                ancestors,
                prefix_atoms,
                shared,
                b_atom,
                m_atom,
            } => {
                let member_bytes = self.entries[member].bytes.clone();
                let member_identifier = self.entries[member].identifier.clone();
                let old_address = self.entries[member].address.clone();

                let mut cursor = base;
                self.trie[base as usize].content = None;
                for s in &shared {
                    let next = self.new_node();
                    self.trie[cursor as usize].children.insert(*s, next);
                    self.trie[next as usize].count = 2;
                    cursor = next;
                }
                let m_marker = self.new_node();
                self.trie[m_marker as usize].content = Some(member);
                self.trie[m_marker as usize].count = 1;
                let b_marker = self.new_node();
                self.trie[b_marker as usize].content = Some(PLACEHOLDER);
                self.trie[b_marker as usize].count = 1;
                self.trie[cursor as usize].children.insert(m_atom, m_marker);
                self.trie[cursor as usize].children.insert(b_atom, b_marker);
                for &ancestor in &ancestors {
                    self.trie[ancestor as usize].count += 1;
                }

                let mut m_atoms = prefix_atoms.clone();
                m_atoms.extend(shared.iter().copied());
                m_atoms.push(m_atom);
                let m_address = NeighborhoodAddress::new(m_atoms).expect("nonempty");
                let mut b_atoms = prefix_atoms;
                b_atoms.extend(shared.iter().copied());
                b_atoms.push(b_atom);
                let b_address = NeighborhoodAddress::new(b_atoms).expect("nonempty");

                if self.entries[member].address != m_address {
                    self.entries[member].address = m_address.clone();
                }
                self.entry_nodes[member] = m_marker;
                let changes = vec![AddressChange {
                    bytes: member_bytes,
                    identifier: member_identifier,
                    old: old_address,
                    new: m_address,
                }];
                (changes, b_address, b_marker)
            }
        };

        self.entries.insert(
            pos,
            NeighborhoodEntry {
                address: new_address.clone(),
                bytes,
                identifier,
            },
        );
        self.entry_nodes.insert(pos, b_marker);

        for node in &mut self.trie {
            match node.content {
                Some(PLACEHOLDER) => node.content = Some(pos),
                Some(c) if c != PLACEHOLDER && c >= pos => node.content = Some(c + 1),
                _ => {}
            }
        }
        let mut shifts = Vec::new();
        for (id, &value) in &self.identifier_index {
            if value >= pos {
                shifts.push((id.clone(), value + 1));
            }
        }
        for (id, value) in shifts {
            self.identifier_index.insert(id, value);
        }
        if let Some(id) = self.entries[pos].identifier.as_ref() {
            self.identifier_index.insert(id.clone(), pos);
        }
        changes.sort_by(|a, b| a.bytes.cmp(&b.bytes));
        InsertResult {
            status: InsertStatus::Inserted,
            address: new_address,
            changes,
        }
    }

    fn remove_inner(&mut self, idx: usize) -> RemoveResult {
        let removed = self.entries[idx].clone();
        let address = removed.address.clone();

        let mut path = vec![0u32];
        let mut edges = vec![AtomId(0)];
        let mut node = 0u32;
        for atom in &address.atoms {
            let &child = self.trie[node as usize]
                .children
                .get(atom)
                .expect("marker path must be materialized");
            path.push(child);
            edges.push(*atom);
            node = child;
        }
        debug_assert_eq!(self.trie[node as usize].content, Some(idx));

        for &n in &path {
            self.trie[n as usize].count -= 1;
        }
        let parent = path[path.len() - 2];
        self.trie[parent as usize]
            .children
            .remove(&address.atoms[address.len() - 1]);
        self.free_node(*path.last().expect("path is nonempty"));

        let mut changes = Vec::<AddressChange>::new();
        for k in (1..path.len() - 1).rev() {
            let n = path[k];
            let trie_node = &self.trie[n as usize];
            if trie_node.count == 0 {
                let p = path[k - 1];
                let e = edges[k];
                self.trie[p as usize].children.remove(&e);
                self.free_node(n);
            } else if trie_node.count == 1
                && trie_node.content.is_none()
                && trie_node.children.len() == 1
            {
                let member = self
                    .sole_member_at(n)
                    .expect("single-child count-1 node must reach a marker");
                let old = self.entries[member].address.clone();
                let new_atoms = edges[1..=k].to_vec();
                let new = NeighborhoodAddress::new(new_atoms).expect("at least one atom");

                let first_child = *self.trie[n as usize]
                    .children
                    .values()
                    .next()
                    .expect("exactly one child");
                let mut marker = first_child;
                let mut chain = vec![first_child];
                loop {
                    if self.trie[marker as usize].content.is_some() {
                        break;
                    }
                    marker = *self.trie[marker as usize]
                        .children
                        .values()
                        .next()
                        .expect("marker chain must be single-child");
                    chain.push(marker);
                }
                self.trie[n as usize].content = Some(member);
                self.trie[n as usize].children.clear();
                self.trie[n as usize].count = 1;
                self.entry_nodes[member] = n;
                for freed in chain {
                    self.free_node(freed);
                }
                if self.entries[member].address != new {
                    self.entries[member].address = new.clone();
                    changes.push(AddressChange {
                        bytes: self.entries[member].bytes.clone(),
                        identifier: self.entries[member].identifier.clone(),
                        old,
                        new,
                    });
                }
            }
        }

        self.entries.remove(idx);
        self.entry_nodes.remove(idx);
        if let Some(id) = &removed.identifier {
            self.identifier_index.remove(id);
        }
        for node in &mut self.trie {
            if let Some(c) = node.content
                && c > idx
            {
                node.content = Some(c - 1);
            }
        }
        self.identifier_index = self
            .identifier_index
            .iter()
            .map(|(k, &v)| (k.clone(), if v > idx { v - 1 } else { v }))
            .collect();

        if self.entries.is_empty() {
            self.trie = vec![TrieNode::empty()];
            self.free_nodes.clear();
            self.entry_nodes.clear();
        } else if self.entries.len() == 1 {
            let member = 0;
            let sole_bytes = self.entries[member].bytes.clone();
            let first = self
                .scheme
                .address(&sole_bytes)
                .next()
                .expect("scheme streams are infinite");
            let single = NeighborhoodAddress::new(vec![first]).expect("nonempty");
            if self.entries[member].address != single {
                let old = self.entries[member].address.clone();
                let identifier = self.entries[member].identifier.clone();
                self.entries[member].address = single.clone();
                changes.push(AddressChange {
                    bytes: sole_bytes,
                    identifier,
                    old,
                    new: single,
                });
            }
            let leaf = self.new_node();
            self.trie[leaf as usize].content = Some(member);
            self.trie[leaf as usize].count = 1;
            self.trie[0].children.clear();
            self.trie[0].children.insert(first, leaf);
            self.trie[0].count = 1;
            self.free_nodes.clear();
            self.entry_nodes = vec![leaf];
        }

        changes.sort_by(|a, b| a.bytes.cmp(&b.bytes));
        RemoveResult { removed, changes }
    }

    fn sole_member_at(&self, mut node: u32) -> Option<usize> {
        loop {
            if let Some(content) = self.trie[node as usize].content {
                return Some(content);
            }
            if self.trie[node as usize].children.len() != 1 {
                return None;
            }
            node = *self.trie[node as usize]
                .children
                .values()
                .next()
                .expect("single child");
        }
    }

    fn new_node(&mut self) -> u32 {
        if let Some(slot) = self.free_nodes.pop() {
            self.trie[slot as usize] = TrieNode::empty();
            slot
        } else {
            let idx = self.trie.len() as u32;
            self.trie.push(TrieNode::empty());
            idx
        }
    }

    fn free_node(&mut self, node: u32) {
        self.trie[node as usize] = TrieNode::empty();
        self.free_nodes.push(node);
    }
}

impl PartialEq for Neighborhood {
    fn eq(&self, other: &Self) -> bool {
        self.scheme == other.scheme && self.entries == other.entries
    }
}
impl Eq for Neighborhood {}

/// Deterministic, index-independent structural form of a trie, for validation.
type TrieFormNode = (Vec<(AtomId, u32)>, Option<usize>, usize);
type TrieForm = Vec<TrieFormNode>;

fn canonical_trie_form(trie: &[TrieNode]) -> TrieForm {
    let mut map = vec![Option::<u32>::None; trie.len()];
    let mut form = Vec::new();
    let mut pending = std::collections::VecDeque::from([0u32]);
    map[0] = Some(0);
    let mut next = 1u32;
    while let Some(n) = pending.pop_front() {
        let tn = &trie[n as usize];
        let mut children = Vec::new();
        for (&atom, &child) in &tn.children {
            let id = match map[child as usize] {
                Some(id) => id,
                None => {
                    let id = next;
                    next += 1;
                    map[child as usize] = Some(id);
                    id
                }
            };
            children.push((atom, id));
            pending.push_back(child);
        }
        form.push((children, tn.content, tn.count));
    }
    form
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{ArtifactError, decode_neighborhood, encode_neighborhood};
    use crate::codec::ContentCodecDescriptor;

    fn small_scheme() -> Scheme {
        Scheme::fallback_only(11, ContentCodecDescriptor::BinaryMsb0).expect("valid small scheme")
    }

    fn tiny_scheme() -> Scheme {
        Scheme::fallback_only(3, ContentCodecDescriptor::BinaryMsb0).expect("valid tiny scheme")
    }

    /// Brute-force reference: `1 + max LCP` (exactly one atom for singletons).
    fn brute_addresses(scheme: &Scheme, members: &[Vec<u8>], mine: &[u8]) -> Vec<AtomId> {
        if members.len() == 1 {
            return vec![scheme.address(mine).next().expect("stream nonempty")];
        }
        let mut max_lcp = 0usize;
        for other in members.iter().filter(|o| o.as_slice() != mine) {
            let mut a = scheme.address(mine);
            let mut b = scheme.address(other);
            let mut lcp = 0usize;
            loop {
                match (a.next(), b.next()) {
                    (Some(x), Some(y)) if x == y => lcp += 1,
                    _ => break,
                }
            }
            max_lcp = max_lcp.max(lcp);
        }
        scheme.address(mine).take(max_lcp + 1).collect()
    }

    type Snapshot = Vec<(Vec<u8>, Vec<AtomId>, Option<Vec<u8>>)>;

    fn snapshot(n: &Neighborhood) -> Snapshot {
        n.entries()
            .map(|e| {
                (
                    e.bytes().to_vec(),
                    e.address().atoms().to_vec(),
                    e.identifier().map(<[u8]>::to_vec),
                )
            })
            .collect()
    }

    fn tiny_permutations(n: usize) -> Vec<Vec<usize>> {
        fn rec(k: usize, set: &mut Vec<usize>, acc: &mut Vec<Vec<usize>>) {
            if k == set.len() {
                acc.push(set.clone());
                return;
            }
            for i in k..set.len() {
                set.swap(i, k);
                rec(k + 1, set, acc);
            }
        }
        let mut set: Vec<usize> = (0..n).collect();
        let mut acc = Vec::new();
        rec(0, &mut set, &mut acc);
        acc
    }

    fn permute<T: Clone>(v: &[T], perm: &[usize]) -> Vec<T> {
        perm.iter().map(|&i| v[i].clone()).collect()
    }

    #[test]
    fn empty_neighborhood_validates() {
        let n = Neighborhood::new(small_scheme()).unwrap();
        assert!(n.is_empty());
        assert_eq!(n.len(), 0);
        n.validate().unwrap();
    }

    #[test]
    fn first_distinct_content_gets_one_atom() {
        let mut n = Neighborhood::new(small_scheme()).unwrap();
        let res = n.insert(b"alpha".to_vec()).unwrap();
        assert_eq!(res.status, InsertStatus::Inserted);
        assert_eq!(res.address.len(), 1);
        assert!(res.changes.is_empty());
        assert_eq!(
            res.address.atoms(),
            brute_addresses(n.scheme(), &[b"alpha".to_vec()], b"alpha")
        );
    }

    #[test]
    fn colliding_contents_extend_to_shortest_distinct_prefixes() {
        let mut n = Neighborhood::new(tiny_scheme()).unwrap();
        let a = b"aa".to_vec();
        let b = b"bb".to_vec();
        n.insert(a.clone()).unwrap();
        n.insert(b.clone()).unwrap();
        let got_a = n.get_by_bytes(&a).unwrap().address().atoms().to_vec();
        let got_b = n.get_by_bytes(&b).unwrap().address().atoms().to_vec();
        assert_eq!(
            got_a,
            brute_addresses(n.scheme(), &[a.clone(), b.clone()], &a)
        );
        assert_eq!(
            got_b,
            brute_addresses(n.scheme(), &[a.clone(), b.clone()], &b)
        );
        assert!(got_a.len() > 1 || got_b.len() > 1);
        n.validate().unwrap();
    }

    #[test]
    fn inserting_collision_lengthens_affected_entries_only() {
        let mut n = Neighborhood::new(tiny_scheme()).unwrap();
        let members = [b"aaaa".to_vec(), b"aaab".to_vec(), b"bbbb".to_vec()];
        for m in &members {
            n.insert(m.clone()).unwrap();
        }
        let before: Vec<(Vec<u8>, Vec<AtomId>)> =
            snapshot(&n).into_iter().map(|(b, a, _)| (b, a)).collect();
        let blocker = b"aacc".to_vec();
        n.insert(blocker.clone()).unwrap();
        let mut all: Vec<Vec<u8>> = before.iter().map(|(b, _)| b.clone()).collect();
        all.push(blocker.clone());
        for (bytes, address) in &before {
            let expect = brute_addresses(n.scheme(), &all, bytes);
            let actual = n.get_by_bytes(bytes).unwrap().address().atoms().to_vec();
            if bytes.as_slice() == b"aaaa" || bytes.as_slice() == b"aaab" {
                assert!(actual.len() >= address.len(), "{bytes:?} lengthened");
            }
            assert_eq!(actual, expect);
        }
        // An entry that shares no materialized path with the blocker is untouched.
        assert_eq!(
            n.get_by_bytes(b"bbbb").unwrap().address().atoms(),
            before
                .iter()
                .find(|(b, _)| b == b"bbbb")
                .map(|(_, a)| a.as_slice())
                .unwrap()
        );
    }

    #[test]
    fn noncolliding_entries_do_not_change() {
        let mut n = Neighborhood::new(small_scheme()).unwrap();
        n.insert(b"a".to_vec()).unwrap();
        n.insert(b"bb".to_vec()).unwrap();
        let before = snapshot(&n);
        // Each inserted entry has a distinct byte length, so its gamma length
        // prefix diverges from every existing stream within a couple of atoms;
        // none of the existing addresses can be lengthened.
        n.insert(b"ccc".to_vec()).unwrap();
        n.insert(b"dddd".to_vec()).unwrap();
        for (bytes, address, _) in &before {
            assert_eq!(
                n.get_by_bytes(bytes).unwrap().address().atoms(),
                address.as_slice(),
                "{bytes:?}"
            );
        }
        assert_eq!(n.get_by_bytes(b"ccc").unwrap().address().len(), 5);
        n.validate().unwrap();
    }

    #[test]
    fn every_permutation_produces_identical_final_addresses() {
        let set: Vec<Vec<u8>> = (0..5)
            .map(|i| format!("payload-{i}").into_bytes())
            .collect();
        let mut reference: Option<Vec<_>> = None;
        for perm in tiny_permutations(set.len()) {
            let mut n = Neighborhood::new(tiny_scheme()).unwrap();
            for bytes in permute(&set, &perm) {
                n.insert(bytes).unwrap();
            }
            let got = snapshot(&n);
            match &reference {
                None => reference = Some(got),
                Some(expected) => assert_eq!(expected, &got),
            }
        }
    }

    #[test]
    fn duplicate_bytes_insertion_is_idempotent() {
        let mut n = Neighborhood::new(small_scheme()).unwrap();
        let first = n.insert(b"x".to_vec()).unwrap();
        let second = n.insert(b"x".to_vec()).unwrap();
        assert_eq!(second.status, InsertStatus::Existing);
        assert_eq!(second.address, first.address);
        assert!(second.changes.is_empty());
        assert_eq!(n.len(), 1);
    }

    #[test]
    fn identifiers_never_affect_addresses() {
        let rows = [b"foo".to_vec(), b"bar".to_vec(), b"baz".to_vec()];
        let mut plain = Neighborhood::new(tiny_scheme()).unwrap();
        let mut tagged = Neighborhood::new(tiny_scheme()).unwrap();
        for (i, bytes) in rows.iter().enumerate() {
            plain.insert(bytes.clone()).unwrap();
            tagged
                .insert_with_identifier(bytes.clone(), format!("id-{i}").into_bytes())
                .unwrap();
        }
        let p = snapshot(&plain);
        let t = snapshot(&tagged);
        for ((pb, pa, _), (tb, ta, _)) in p.iter().zip(&t) {
            assert_eq!(pb, tb);
            assert_eq!(pa, ta);
        }
    }

    #[test]
    fn duplicate_identifier_on_different_content_rejected_transactionally() {
        let mut n = Neighborhood::new(small_scheme()).unwrap();
        n.insert_with_identifier(b"one".to_vec(), b"shared".to_vec())
            .unwrap();
        let before = n.clone();
        let err = n
            .insert_with_identifier(b"two".to_vec(), b"shared".to_vec())
            .unwrap_err();
        assert!(matches!(
            err,
            NeighborhoodError::IdentifierAlreadyAssigned(_)
        ));
        assert_eq!(n, before);
    }

    #[test]
    fn conflicting_identifiers_for_identical_content_rejected() {
        let mut n = Neighborhood::new(small_scheme()).unwrap();
        n.insert_with_identifier(b"same".to_vec(), b"id-a".to_vec())
            .unwrap();
        let before = n.clone();
        let err = n
            .insert_with_identifier(b"same".to_vec(), b"id-b".to_vec())
            .unwrap_err();
        assert!(matches!(
            err,
            NeighborhoodError::ConflictingIdentifierForContent(_)
        ));
        assert_eq!(n, before);
        assert_eq!(n.get_by_identifier(b"id-a").unwrap().bytes(), b"same");
    }

    #[test]
    fn attaching_identifier_to_existing_content_is_existing() {
        let mut n = Neighborhood::new(small_scheme()).unwrap();
        n.insert(b"k".to_vec()).unwrap();
        let before = n.get_by_bytes(b"k").unwrap().address().clone();
        let res = n
            .insert_with_identifier(b"k".to_vec(), b"late-id".to_vec())
            .unwrap();
        assert_eq!(res.status, InsertStatus::Existing);
        assert!(res.changes.is_empty());
        assert_eq!(n.get_by_bytes(b"k").unwrap().address(), &before);
        assert_eq!(n.get_by_identifier(b"late-id").unwrap().bytes(), b"k");
        n.validate().unwrap();
    }

    #[test]
    fn removal_restores_canonical_shortest_prefixes_and_reports_changes() {
        let mut n = Neighborhood::new(tiny_scheme()).unwrap();
        let members: Vec<Vec<u8>> = (0..5).map(|i| format!("m{i}").into_bytes()).collect();
        for bytes in &members {
            n.insert(bytes.clone()).unwrap();
        }
        for b in &members {
            assert_eq!(
                n.get_by_bytes(b).unwrap().address().atoms(),
                &brute_addresses(n.scheme(), &members, b)[..]
            );
        }
        let removed = members[2].clone();
        let result = n.remove_by_bytes(&removed).unwrap().unwrap();
        assert_eq!(result.removed.bytes(), &removed[..]);
        let remaining: Vec<Vec<u8>> = members
            .iter()
            .filter(|b| b.as_slice() != removed.as_slice())
            .cloned()
            .collect();
        for b in &remaining {
            assert_eq!(
                n.get_by_bytes(b).unwrap().address().atoms(),
                &brute_addresses(n.scheme(), &remaining, b)[..],
                "{b:?}"
            );
        }
        for change in &result.changes {
            assert!(remaining.contains(&change.bytes));
            assert!(change.new.len() <= change.old.len());
        }
        n.validate().unwrap();
    }

    #[test]
    fn removal_shortens_one_member_to_single_atom() {
        let mut n = Neighborhood::new(small_scheme()).unwrap();
        n.insert(b"solo".to_vec()).unwrap();
        n.insert(b"duo".to_vec()).unwrap();
        n.remove_by_bytes(b"solo").unwrap().unwrap();
        assert_eq!(n.get_by_bytes(b"duo").unwrap().address().len(), 1);
        n.validate().unwrap();
    }

    #[test]
    fn arbitrary_prefix_resolution_unique_and_ambiguous() {
        let mut n = Neighborhood::new(tiny_scheme()).unwrap();
        n.insert(b"aaaa".to_vec()).unwrap();
        n.insert(b"aaab".to_vec()).unwrap();
        n.insert(b"bbbb".to_vec()).unwrap();

        let mut shared = Vec::new();
        let (mut a, mut b) = (n.scheme().address(b"aaaa"), n.scheme().address(b"aaab"));
        while let (Some(x), Some(y)) = (a.next(), b.next()) {
            if x != y {
                break;
            }
            shared.push(x);
        }
        assert!(!shared.is_empty());
        assert!(matches!(
            n.resolve(&NeighborhoodAddress::new(shared.clone()).unwrap()),
            Ok(Resolution::Ambiguous { matches: 2 })
        ));

        let full_a = n.get_by_bytes(b"aaaa").unwrap().address().clone();
        assert!(matches!(n.resolve(&full_a), Ok(Resolution::Unique(e)) if e.bytes() == b"aaaa"));
    }

    #[test]
    fn prefix_mismatching_stream_continuation_is_notfound() {
        let mut n = Neighborhood::new(tiny_scheme()).unwrap();
        n.insert(b"aaaa".to_vec()).unwrap();
        n.insert(b"aaab".to_vec()).unwrap();
        let addr = n.get_by_bytes(b"aaaa").unwrap().address().clone();
        let next = n.scheme().address(b"aaaa").nth(addr.len()).unwrap();
        let mut bogus = AtomId((next.0 + 1) % 3);
        if bogus == next {
            bogus = AtomId((next.0 + 2) % 3);
        }
        let mut extended = addr.atoms().to_vec();
        extended.push(bogus);
        assert!(matches!(
            n.resolve(&NeighborhoodAddress::new(extended).unwrap()),
            Ok(Resolution::NotFound)
        ));
        // The genuine continuation resolves.
        let mut genuine = addr.atoms().to_vec();
        genuine.push(next);
        assert!(matches!(
            n.resolve(&NeighborhoodAddress::new(genuine).unwrap()),
            Ok(Resolution::Unique(e)) if e.bytes() == b"aaaa"
        ));
    }

    #[test]
    fn old_shorter_address_becomes_ambiguous_after_colliding_insertion() {
        let mut n = Neighborhood::new(tiny_scheme()).unwrap();
        n.insert(b"zapp".to_vec()).unwrap();
        let old = n.get_by_bytes(b"zapp").unwrap().address().clone();
        assert_eq!(old.len(), 1);
        // A sibling of identical byte length shares the whole Elias-gamma length
        // prefix, so its stream begins with the same first atom.
        n.insert(b"zbur".to_vec()).unwrap();
        assert!(matches!(
            n.resolve(&old),
            Ok(Resolution::Ambiguous { matches: 2 })
        ));
    }

    #[test]
    fn old_longer_address_still_resolves_after_deletion_shortens() {
        let mut n = Neighborhood::new(tiny_scheme()).unwrap();
        n.insert(b"qaaa".to_vec()).unwrap();
        n.insert(b"qabb".to_vec()).unwrap();
        n.insert(b"qccc".to_vec()).unwrap();
        // Record qccc's address, remove the two entries it collides with so it
        // shortens, then verify the previously-issued longer address still
        // resolves uniquely to qccc.
        let before = n.clone();
        let long = before.get_by_bytes(b"qccc").unwrap().address().clone();
        n.remove_by_bytes(b"qaaa").unwrap().unwrap();
        n.remove_by_bytes(b"qabb").unwrap().unwrap();
        assert!(n.get_by_bytes(b"qccc").unwrap().address().len() < long.len());
        assert!(matches!(
            n.resolve(&long),
            Ok(Resolution::Unique(e)) if e.bytes() == b"qccc"
        ));
    }

    #[test]
    fn from_rows_is_permutation_independent() {
        let rows: Vec<NeighborhoodRow> = (0..7)
            .map(|i| NeighborhoodRow {
                bytes: format!("row-{i}").into_bytes(),
                identifier: None,
            })
            .collect();
        let base = Neighborhood::from_rows(tiny_scheme(), rows.clone()).unwrap();
        for perm in tiny_permutations(rows.len()) {
            let built = Neighborhood::from_rows(tiny_scheme(), permute(&rows, &perm)).unwrap();
            assert_eq!(snapshot(&built), snapshot(&base));
        }
    }

    #[test]
    fn merge_equals_fresh_construction_over_union() {
        let mut left = Neighborhood::new(tiny_scheme()).unwrap();
        left.insert(b"merge-a".to_vec()).unwrap();
        left.insert(b"merge-b".to_vec()).unwrap();
        let mut right = Neighborhood::new(tiny_scheme()).unwrap();
        right.insert(b"merge-b".to_vec()).unwrap();
        right.insert(b"merge-c".to_vec()).unwrap();

        let union: std::collections::BTreeMap<Vec<u8>, Option<Vec<u8>>> = snapshot(&left)
            .into_iter()
            .chain(snapshot(&right))
            .map(|(b, _, id)| (b, id))
            .collect();
        let merged = Neighborhood::from_rows(
            tiny_scheme(),
            union
                .into_iter()
                .map(|(bytes, identifier)| NeighborhoodRow { bytes, identifier }),
        )
        .unwrap();

        let mut subject = Neighborhood::new(tiny_scheme()).unwrap();
        subject.insert(b"merge-a".to_vec()).unwrap();
        subject.insert(b"merge-b".to_vec()).unwrap();
        subject.merge(&right).unwrap();
        subject.validate().unwrap();
        assert_eq!(snapshot(&subject), snapshot(&merged));
    }

    #[test]
    fn scheme_mismatch_merge_rejected_without_mutation() {
        let mut n = Neighborhood::new(small_scheme()).unwrap();
        n.insert(b"x".to_vec()).unwrap();
        let other = Neighborhood::new(tiny_scheme()).unwrap();
        let before = n.clone();
        let err = n.merge(&other).unwrap_err();
        assert_eq!(err, NeighborhoodError::SchemeMismatch);
        assert_eq!(n, before);
    }

    #[test]
    fn merge_identifier_conflict_is_transactional() {
        let mut left = Neighborhood::new(small_scheme()).unwrap();
        left.insert_with_identifier(b"alpha".to_vec(), b"row-1".to_vec())
            .unwrap();
        let mut right = Neighborhood::new(small_scheme()).unwrap();
        right
            .insert_with_identifier(b"beta".to_vec(), b"row-1".to_vec())
            .unwrap();
        let before = left.clone();
        let err = left.merge(&right).unwrap_err();
        assert!(matches!(
            err,
            NeighborhoodError::IdentifierAlreadyAssigned(_)
        ));
        assert_eq!(left, before);
    }

    #[test]
    fn validate_detects_corrupted_entries() {
        let mut n = Neighborhood::new(small_scheme()).unwrap();
        n.insert(b"corrupt-a".to_vec()).unwrap();
        n.insert(b"corrupt-b".to_vec()).unwrap();
        if let Some(entry) = n.entries.get_mut(0) {
            let mut atoms = entry.address.atoms.clone();
            atoms.push(AtomId(0));
            entry.address = NeighborhoodAddress::new(atoms).unwrap();
        }
        assert!(matches!(
            n.validate(),
            Err(NeighborhoodError::InvalidState(_))
        ));
    }

    #[test]
    fn validate_detects_corrupted_trie() {
        let mut n = Neighborhood::new(small_scheme()).unwrap();
        n.insert(b"trie-a".to_vec()).unwrap();
        n.insert(b"trie-b".to_vec()).unwrap();
        for node in &mut n.trie {
            node.count = node.count.saturating_add(1_000_000);
        }
        assert!(matches!(
            n.validate(),
            Err(NeighborhoodError::InvalidState(_))
        ));
    }

    #[test]
    fn rebuild_restores_canonical_derived_state() {
        let mut n = Neighborhood::new(small_scheme()).unwrap();
        n.insert(b"rebuild-a".to_vec()).unwrap();
        n.insert(b"rebuild-b".to_vec()).unwrap();
        n.insert(b"rebuild-c".to_vec()).unwrap();
        let canonical = snapshot(&n);
        for (i, entry) in n.entries.iter_mut().enumerate() {
            entry.address = NeighborhoodAddress::new(vec![AtomId(i as u32 + 1)]).unwrap();
        }
        n.trie = vec![TrieNode::empty()];
        n.free_nodes.clear();
        n.entry_nodes.clear();
        n.identifier_index.clear();
        let changes = n.rebuild().unwrap();
        assert!(!changes.is_empty());
        assert_eq!(snapshot(&n), canonical);
        n.validate().unwrap();
    }

    #[test]
    fn artifact_round_trip_preserves_neighborhood() {
        let mut n = Neighborhood::new(small_scheme()).unwrap();
        n.insert_with_identifier(b"art-a".to_vec(), b"id-a".to_vec())
            .unwrap();
        n.insert(b"art-b".to_vec()).unwrap();
        n.insert_with_identifier(b"art-c".to_vec(), b"id-c".to_vec())
            .unwrap();
        let bytes = encode_neighborhood(&n).unwrap();
        let decoded = decode_neighborhood(&bytes).unwrap();
        assert_eq!(snapshot(&n), snapshot(&decoded));
        decoded.validate().unwrap();
    }

    #[test]
    fn artifact_bytes_identical_across_insertion_permutations() {
        let set: Vec<Vec<u8>> = (0..5).map(|i| format!("perm-{i}").into_bytes()).collect();
        let mut encoded = Vec::new();
        for perm in tiny_permutations(set.len()) {
            let mut n = Neighborhood::new(small_scheme()).unwrap();
            for bytes in permute(&set, &perm) {
                n.insert(bytes).unwrap();
            }
            encoded.push(encode_neighborhood(&n).unwrap());
        }
        for other in &encoded[1..] {
            assert_eq!(&encoded[0], other);
        }
    }

    #[test]
    fn decoder_rejects_bad_magic_truncation_and_trailing() {
        let n = Neighborhood::from_rows(
            small_scheme(),
            [NeighborhoodRow {
                bytes: b"magic".to_vec(),
                identifier: None,
            }],
        )
        .unwrap();
        let bytes = encode_neighborhood(&n).unwrap();
        let mut bad = bytes.clone();
        bad[0] = b'X';
        assert!(matches!(
            decode_neighborhood(&bad),
            Err(ArtifactError::BadMagic)
        ));
        for cut in [0, 1, 7, bytes.len() - 1] {
            assert!(matches!(
                decode_neighborhood(&bytes[..cut]),
                Err(ArtifactError::Truncated) | Err(ArtifactError::BadMagic)
            ));
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(matches!(
            decode_neighborhood(&trailing),
            Err(ArtifactError::TrailingBytes)
        ));
    }

    #[test]
    fn decoder_rejects_noncanonical_addresses() {
        let n = Neighborhood::from_rows(
            small_scheme(),
            [NeighborhoodRow {
                bytes: b"noncanonical".to_vec(),
                identifier: None,
            }],
        )
        .unwrap();
        let bytes = encode_neighborhood(&n).unwrap();
        // Corrupt the last serialized address atom into another capacity-valid
        // atom so the canonical comparison must fail.
        let last = bytes.len() - 1;
        let mut corrupted = bytes.clone();
        corrupted[last] = if corrupted[last] == 0 {
            1
        } else {
            corrupted[last] - 1
        };
        assert!(matches!(
            decode_neighborhood(&corrupted),
            Err(ArtifactError::InvalidNeighborhood(_))
        ));
    }

    #[test]
    fn custom_schemes_work_not_only_embedded_default() {
        let mut n = Neighborhood::new(tiny_scheme()).unwrap();
        n.insert(b"c1".to_vec()).unwrap();
        n.insert(b"c2".to_vec()).unwrap();
        n.insert(b"c3".to_vec()).unwrap();
        n.validate().unwrap();
        let bytes = encode_neighborhood(&n).unwrap();
        let decoded = decode_neighborhood(&bytes).unwrap();
        assert_eq!(decoded.scheme(), n.scheme());
        assert_eq!(decoded.len(), n.len());
    }

    #[test]
    fn canonical_scheme_neighborhood_works() {
        let mut n = Neighborhood::canonical();
        n.insert(b"north".to_vec()).unwrap();
        n.insert(b"south".to_vec()).unwrap();
        n.insert(b"east".to_vec()).unwrap();
        n.insert(b"west".to_vec()).unwrap();
        assert_eq!(n.len(), 4);
        n.validate().unwrap();
        let bytes = encode_neighborhood(&n).unwrap();
        assert_eq!(
            snapshot(&decode_neighborhood(&bytes).unwrap()),
            snapshot(&n)
        );
    }

    #[test]
    fn empty_addresses_are_rejected_at_construction() {
        assert!(matches!(
            NeighborhoodAddress::new(vec![]),
            Err(NeighborhoodError::EmptyAddress)
        ));
        let mut n = Neighborhood::new(small_scheme()).unwrap();
        n.insert(b"cap".to_vec()).unwrap();
        let out_of_range = NeighborhoodAddress::new(vec![AtomId(u32::MAX)]).unwrap();
        assert!(matches!(
            n.resolve(&out_of_range),
            Err(NeighborhoodError::AtomOutOfCapacity { .. })
        ));
    }

    #[test]
    fn resolve_prefix_longer_than_canonical_address() {
        let mut n = Neighborhood::new(small_scheme()).unwrap();
        n.insert(b"long-a".to_vec()).unwrap();
        n.insert(b"long-b".to_vec()).unwrap();
        let entry = n.get_by_bytes(b"long-a").unwrap();
        let extended: Vec<AtomId> = n
            .scheme()
            .address(b"long-a")
            .take(entry.address().len() + 2)
            .collect();
        let addr = NeighborhoodAddress::new(extended).unwrap();
        assert!(matches!(n.resolve(&addr), Ok(Resolution::Unique(e)) if e.bytes() == b"long-a"));
    }

    #[test]
    fn properties_hold_for_deterministic_sets() {
        let mut rng = Lcg::new(0x5eed_2026);
        let mut rounds = 0;
        while rounds < 200 {
            let count = 1 + rng.next_u64() as usize % 6;
            let mut set: Vec<Vec<u8>> = (0..count)
                .map(|_| {
                    let len = 3 + rng.next_u64() as usize % 4;
                    (0..len).map(|_| (rng.next_u64() & 0xff) as u8).collect()
                })
                .collect();
            set.sort();
            set.dedup();
            if set.is_empty() {
                continue;
            }
            rounds += 1;

            let mut n = Neighborhood::new(tiny_scheme()).unwrap();
            for b in &set {
                n.insert(b.clone()).unwrap();
            }
            n.validate().unwrap();

            let addrs: Vec<Vec<AtomId>> = set
                .iter()
                .map(|b| n.get_by_bytes(b).unwrap().address().atoms().to_vec())
                .collect();
            for (i, a) in addrs.iter().enumerate() {
                for (j, other) in addrs.iter().enumerate() {
                    if i != j {
                        assert!(!a.starts_with(other), "prefix-free violated");
                        assert!(!other.starts_with(a));
                    }
                }
                let entry = n.get_by_bytes(&set[i]).unwrap();
                let stream: Vec<AtomId> = n
                    .scheme()
                    .address(&set[i])
                    .take(entry.address().len())
                    .collect();
                assert_eq!(stream, *a);
                assert_eq!(
                    entry.address().atoms(),
                    &brute_addresses(n.scheme(), &set.to_vec(), &set[i])[..]
                );
                // Minimality: shortening any len>1 address makes it non-unique
                // (or resolves to a different content).
                if entry.address().len() > 1 {
                    let shorter = NeighborhoodAddress::new(a[..a.len() - 1].to_vec()).unwrap();
                    match n.resolve(&shorter).unwrap() {
                        Resolution::Unique(other) => {
                            assert_ne!(other.bytes(), set[i].as_slice());
                        }
                        Resolution::Ambiguous { .. } | Resolution::NotFound => {}
                    }
                }
            }
        }
    }

    #[test]
    fn removal_property_across_sets() {
        let mut rng = Lcg::new(0xabc_123);
        let mut rounds = 0;
        while rounds < 120 {
            rounds += 1;
            let count = 2 + rng.next_u64() as usize % 5;
            let mut set: Vec<Vec<u8>> = (0..count)
                .map(|_| {
                    let len = 4 + rng.next_u64() as usize % 3;
                    (0..len).map(|_| (rng.next_u64() & 0xff) as u8).collect()
                })
                .collect();
            set.sort();
            set.dedup();
            if set.len() < 2 {
                continue;
            }
            let mut n = Neighborhood::new(tiny_scheme()).unwrap();
            for b in &set {
                n.insert(b.clone()).unwrap();
            }
            let victim_index = (rng.next_u64() as usize) % set.len();
            let victim = set[victim_index].clone();
            n.remove_by_bytes(&victim).unwrap().unwrap();
            n.validate().unwrap();
            let mut remaining: Vec<Vec<u8>> = set
                .iter()
                .filter(|b| b.as_slice() != victim.as_slice())
                .cloned()
                .collect();
            remaining.sort();
            remaining.dedup();
            for b in &remaining {
                assert_eq!(
                    n.get_by_bytes(b).unwrap().address().atoms(),
                    &brute_addresses(n.scheme(), &remaining, b)[..],
                    "remaining {b:?}"
                );
            }
        }
    }

    struct Lcg(u64);
    impl Lcg {
        fn new(seed: u64) -> Self {
            Self(seed)
        }
        fn next_u64(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
    }
}
