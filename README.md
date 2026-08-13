# teca

It deterministically generates a collision-minimizing indefinitely extensible content address for whatever bits you put in. Collision-minimization was estimated by algorithmic co-occurrence. The function itself is stateless but determination of unique addresses is stateful because content that previously required one token to address could in the future collide in the first token with other content and require two tokens thereafter.

Not cryptographic. Not meant for adversarial input.

The default lexicon contains 12,359 Unicode-alphanumeric strings that were single tokens across seven representative tokenizers. The source tokenizers are listed in [NOTICE.md](NOTICE.md).

The canonical scheme uses the supplied direct CTM-B9-D12 pair prior and is optimized for early address divergence among co-occurring inputs. This is probably suboptimal for our design constraint (must generalize to all text with meaning), but figuring out other ways of estimating the prior were too hard. My bad. I think you can load your own in but I slopped that hard enough not to be 100% sure.

The canonical scheme and lexicon are embedded in the library and work without setup:

```rust
use teca::default_address;

let atoms: Vec<_> = default_address(input).take(8).collect();
let first_atom = atoms[0];
```

For structural numeric IDs, use `teca::default_atom_ids`. A custom scheme can be paired with a lexicon through `Address::tokens`, which checks that both capacities match.

The binary artifacts are also checked in for inspection and alternate configurations. Load them through the artifact API when a caller supplies a different scheme or lexicon:

```rust
use teca::{decode_lexicon, decode_scheme};

let scheme = decode_scheme(&std::fs::read("data/canonical/teca-canonical-b9-d12-n12359.tecasm")?)?;
let lexicon = decode_lexicon(&std::fs::read("data/canonical/lexicon-cross-tokenizer-all-seven-alphanumeric-v1.tecalx")?)?;
let prefix: Vec<_> = scheme.scheme.address(input).take(8).collect();
```

The offline static cooker is exposed as `teca::cook::cook_static`; the exact expected-cost solver remains available as a library component for small research priors only.

## Neighborhoods

The raw TECA stream is stateless: every call derives the same sequence of `AtomId`s from the same bytes. What makes addresses short and stable is a *neighborhood* — a collection of contents whose canonical addresses are the shortest nonempty prefixes of their own TECA streams that are unique among the members. Uniqueness is therefore relative to a neighborhood, not to the stream alone.

```rust
use teca::Neighborhood;

let mut neighborhood = Neighborhood::canonical();

let a = neighborhood.insert(b"alpha".to_vec())?;
let b = neighborhood.insert_with_identifier(
    b"beta".to_vec(),
    b"database-row-42".to_vec(),
)?;

let entry = neighborhood.get_by_identifier(b"database-row-42").unwrap();
let resolved = neighborhood.resolve(entry.address())?;
```

- Raw TECA streams are stateless; only the neighborhood decides how many atoms make an address unambiguous.
- Addresses may lengthen when colliding content is added: inserting content whose stream shares the existing address of another member extends the older member to a longer unique prefix.
- Addresses may also shorten when content is removed: `remove` restores the shortest possible prefixes for the survivors.
- `resolve` accepts any current unambiguous TECA prefix of a member, so longer historical addresses keep resolving after a shortening.
- The source bytes must be retained, because TECA may need to extend an address later when colliding content arrives; neighborhoods never store historical address aliases.

The neighborhood can be round-tripped through the artifact API (`encode_neighborhood`/`decode_neighborhood`), which yields identical bytes for the same contents and identifiers regardless of insertion order or mutation history.
