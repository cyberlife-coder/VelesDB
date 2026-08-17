use super::*;

/// The compile-time guard `ALL` cannot have: only this crate can enumerate a
/// `non_exhaustive` enum, so only this crate can notice `ALL` going stale.
/// A `match` over `Self` keeps it honest — adding a category without touching
/// `ALL` fails to compile HERE (the defining crate still matches
/// exhaustively), and the assertion documents the contract for readers.
#[test]
fn all_lists_every_category() {
    let mut seen = 0usize;
    for category in ErrorCategory::ALL {
        // Exhaustive on purpose; a new variant must be added to `ALL` and to
        // this match in the same change.
        match category {
            ErrorCategory::InvalidInput | ErrorCategory::NotFound | ErrorCategory::Internal => {
                seen += 1;
            }
        }
    }
    assert_eq!(
        seen,
        ErrorCategory::ALL.len(),
        "ALL carries a duplicate or an unmatched entry"
    );
}
