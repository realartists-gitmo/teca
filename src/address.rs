use crate::lexicon::Lexicon;
use crate::scheme::{Scheme, SchemeError};
use std::fmt;

/// Structural TECA address atom.
///
/// `AtomId` is intentionally independent of any textual lexicon. A lexicon maps
/// IDs to output atoms after the mathematical address has been computed.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct AtomId(pub u32);

impl AtomId {
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Error returned when a scheme and lexicon cannot address the same atom space.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TokenAddressError {
    CapacityMismatch { scheme: u32, lexicon: usize },
}

impl fmt::Display for TokenAddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CapacityMismatch { scheme, lexicon } => {
                write!(
                    f,
                    "scheme capacity {scheme} does not match lexicon capacity {lexicon}"
                )
            }
        }
    }
}

impl std::error::Error for TokenAddressError {}

/// Lazy append-only TECA address iterator.
///
/// The iterator owns only traversal state. The caller bytes and scheme are
/// borrowed and remain canonical semantic input/state.
pub struct Address<'a> {
    pub(crate) scheme: &'a Scheme,
    pub(crate) bytes: &'a [u8],
    pub(crate) next_node: Option<u32>,
    pub(crate) fallback: Option<crate::fallback::FallbackAddress<'a>>,
}

impl<'a> Address<'a> {
    pub(crate) fn new(scheme: &'a Scheme, bytes: &'a [u8]) -> Self {
        Self {
            scheme,
            bytes,
            next_node: scheme.root,
            fallback: None,
        }
    }

    fn next_checked(&mut self) -> Result<AtomId, SchemeError> {
        if let Some(fallback) = self.fallback.as_mut() {
            return Ok(fallback.next_atom());
        }

        let Some(node_id) = self.next_node else {
            let mut fallback = self
                .scheme
                .fallback
                .address(self.bytes, self.scheme.capacity)?;
            let atom = fallback.next_atom();
            self.fallback = Some(fallback);
            return Ok(atom);
        };

        let node = self
            .scheme
            .nodes
            .get(node_id as usize)
            .ok_or(SchemeError::InvalidNode(node_id))?;
        let atom = node
            .action
            .evaluate(self.bytes, self.scheme.codec, self.scheme.capacity)?;
        self.next_node = node.child_for(atom);
        if self.next_node.is_none() {
            // The fallback starts on the *next* atom. The current optimized atom
            // has already been emitted.
        }
        Ok(atom)
    }

    /// Attach a matching lexicon and produce the actual address atoms lazily.
    pub fn tokens<'lexicon>(
        self,
        lexicon: &'lexicon Lexicon,
    ) -> Result<TokenAddress<'a, 'lexicon>, TokenAddressError> {
        if self.scheme.capacity as usize != lexicon.capacity() {
            return Err(TokenAddressError::CapacityMismatch {
                scheme: self.scheme.capacity,
                lexicon: lexicon.capacity(),
            });
        }
        Ok(TokenAddress {
            address: self,
            lexicon,
        })
    }
}

impl Iterator for Address<'_> {
    type Item = AtomId;

    fn next(&mut self) -> Option<Self::Item> {
        // A validated Scheme should make runtime evaluation infallible. Preserve
        // Iterator ergonomics while failing loudly if an invalid hand-built
        // scheme escaped validation.
        Some(
            self.next_checked()
                .unwrap_or_else(|err| panic!("invalid TECA scheme at runtime: {err}")),
        )
    }
}

/// Lazy address iterator that yields the lexicon bytes rather than numeric IDs.
pub struct TokenAddress<'address, 'lexicon> {
    address: Address<'address>,
    lexicon: &'lexicon Lexicon,
}

impl<'address, 'lexicon> Iterator for TokenAddress<'address, 'lexicon> {
    type Item = &'lexicon [u8];

    fn next(&mut self) -> Option<Self::Item> {
        let atom = self.address.next()?;
        Some(
            self.lexicon
                .atom(atom)
                .expect("validated TECA address atom must fit its lexicon"),
        )
    }
}
