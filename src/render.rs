//! Generic boundary-preserving renderers.
//!
//! TECA does not assume arbitrary lexicon atoms can be concatenated safely.

use std::fmt::Write;

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
