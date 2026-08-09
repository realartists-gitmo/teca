use std::sync::OnceLock;

use crate::address::{Address, TokenAddress};
use crate::artifact::{decode_lexicon, decode_scheme};
use crate::lexicon::Lexicon;
use crate::scheme::Scheme;

const DEFAULT_SCHEME_BYTES: &[u8] =
    include_bytes!("../data/canonical/teca-canonical-b9-d12-n12359.tecasm");
const DEFAULT_LEXICON_BYTES: &[u8] =
    include_bytes!("../data/canonical/lexicon-cross-tokenizer-all-seven-alphanumeric-v1.tecalx");

static DEFAULT_SCHEME: OnceLock<Scheme> = OnceLock::new();
static DEFAULT_LEXICON: OnceLock<Lexicon> = OnceLock::new();

/// The canonical embedded addressing scheme.
pub fn default_scheme() -> &'static Scheme {
    DEFAULT_SCHEME.get_or_init(|| {
        decode_scheme(DEFAULT_SCHEME_BYTES)
            .expect("embedded canonical TECA scheme must decode")
            .scheme
    })
}

/// The canonical embedded lexicon corresponding to [`default_scheme`].
pub fn default_lexicon() -> &'static Lexicon {
    DEFAULT_LEXICON.get_or_init(|| {
        decode_lexicon(DEFAULT_LEXICON_BYTES)
            .expect("embedded canonical TECA lexicon must decode")
            .lexicon
    })
}

/// Create an address using the canonical embedded scheme.
pub fn default_atom_ids(bytes: &[u8]) -> Address<'_> {
    default_scheme().address(bytes)
}

/// Create a lazy canonical address iterator yielding the actual lexicon atoms.
pub fn default_address(bytes: &[u8]) -> TokenAddress<'_, 'static> {
    default_scheme()
        .address(bytes)
        .tokens(default_lexicon())
        .expect("embedded canonical scheme and lexicon must have matching capacities")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_defaults_are_usable() {
        assert_eq!(default_scheme().capacity, 12_359);
        assert_eq!(default_lexicon().capacity(), 12_359);
        assert_eq!(default_address(b"hello").take(4).count(), 4);
        assert!(
            default_address(b"hello")
                .next()
                .unwrap()
                .iter()
                .all(u8::is_ascii_alphanumeric)
        );
    }
}
