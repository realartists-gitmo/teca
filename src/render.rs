//! Generic boundary-preserving renderers.
//!
//! TECA does not assume arbitrary lexicon atoms can be concatenated safely.

use std::fmt::Write;

use crate::neighborhood::NeighborhoodAddress;
use crate::{Address, Lexicon, LexiconError};

/// Render a finite atom prefix as an unambiguous textual wire representation.
///
/// Format: `<byte_len>:<hex_bytes>` per atom, separated by `/`.
/// This is intentionally generic and separator-preserving.
pub fn render_text_prefix(
    address: &mut Address<'_>,
    lexicon: &Lexicon,
    depth: usize,
) -> Result<String, LexiconError> {
    let mut out = String::new();
    for i in 0..depth {
        if i != 0 {
            out.push('/');
        }
        let atom = lexicon.atom(
            address
                .next()
                .expect("TECA addresses are indefinitely extensible"),
        )?;
        write!(&mut out, "{}:", atom.len()).expect("writing String cannot fail");
        for byte in atom {
            write!(&mut out, "{byte:02x}").expect("writing String cannot fail");
        }
    }
    Ok(out)
}

/// Render an already-owned [`NeighborhoodAddress`] through a `Lexicon` using the
/// same boundary-preserving representation as [`render_text_prefix`]. Every
/// atom is individually validated against the lexicon capacity by the renderer,
/// so arbitrary lexicon atoms are never implicitly concatenated.
pub fn render_neighborhood_address(
    address: &NeighborhoodAddress,
    lexicon: &Lexicon,
) -> Result<String, LexiconError> {
    let mut out = String::new();
    for (i, atom_id) in address.atoms().iter().enumerate() {
        if i != 0 {
            out.push('/');
        }
        let atom = lexicon.atom(*atom_id)?;
        write!(&mut out, "{}:", atom.len()).expect("writing String cannot fail");
        for byte in atom {
            write!(&mut out, "{byte:02x}").expect("writing String cannot fail");
        }
    }
    Ok(out)
}
