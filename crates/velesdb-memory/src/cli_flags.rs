//! The two refusals every hand-rolled subcommand parser shares.
//!
//! The binary carries no `clap` (two subcommands with a handful of flags do
//! not justify it), so `migrate-embeddings` and `compile-stdin` each walk
//! their own `argv`. What they refuse in common — a flag with no value after
//! it, a flag the subcommand does not know — lives here once, so the two
//! parsers cannot word the same mistake two ways.

/// A command-line flag that cannot be used as given.
///
/// `non_exhaustive`: a new shared refusal adds a variant without a breaking
/// release.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FlagError {
    /// A flag that takes a value was the last argument.
    #[error("{flag} requires a value")]
    MissingValue {
        /// The flag, as typed.
        flag: String,
    },

    /// The subcommand has no such flag.
    #[error("unknown {command} flag {flag:?}")]
    Unknown {
        /// The subcommand that was parsing, e.g. `migrate-embeddings`.
        command: &'static str,
        /// The flag, as typed.
        flag: String,
    },
}

/// The value that follows the flag at `index` in `args`.
///
/// # Errors
/// [`FlagError::MissingValue`] naming `flag` when nothing follows it.
pub fn value_after<'a>(
    args: &'a [String],
    index: usize,
    flag: &str,
) -> Result<&'a String, FlagError> {
    args.get(index + 1).ok_or_else(|| FlagError::MissingValue {
        flag: flag.to_owned(),
    })
}

#[cfg(test)]
#[path = "cli_flags_tests.rs"]
mod tests;
