//! Which ids a store's last writes changed, by revision.
//!
//! Every store and delete of a store moves its revision and records the ids
//! it wrote, under the store's write guard, so a reader holding the read
//! guard twice can ask which ids changed in between: a ranked walk
//! (`collection/search/ranked.rs`) judges again only those, instead of
//! starting over. The log keeps the last [`ChangeLog::CAPACITY`] ids; a
//! reader asking about older revisions learns that it cannot tell.

use std::collections::VecDeque;

/// The ids a store's writes changed, the last [`Self::CAPACITY`] of them,
/// each with the revision of its write.
#[derive(Debug, Default)]
pub(crate) struct ChangeLog {
    revision: u64,
    /// The highest revision some of whose ids the log no longer holds.
    forgotten: u64,
    /// At most [`Self::CAPACITY`] `(revision, id)`, oldest first.
    entries: VecDeque<(u64, u64)>,
}

impl ChangeLog {
    /// How many changed ids the log keeps.
    pub(crate) const CAPACITY: usize = 4096;

    /// How many writes changed the store since it was opened: two reads
    /// with the same answer saw the same store.
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    /// Records one write of `ids` — a failed one included: what it left is
    /// unknown — as the next revision. The log never holds more than
    /// [`Self::CAPACITY`] ids, so its buffer never grows past what that
    /// many need: a write of at least that many replaces every id the log
    /// held with its own last ones.
    pub(crate) fn record<I>(&mut self, ids: I)
    where
        I: IntoIterator<Item = u64>,
        I::IntoIter: ExactSizeIterator,
    {
        self.revision = self.revision.wrapping_add(1);
        let ids = ids.into_iter();
        let written = ids.len();
        let overflow = (self.entries.len() + written).saturating_sub(Self::CAPACITY);
        self.forget_oldest(overflow);
        if written > Self::CAPACITY {
            // This write's own first ids are not kept either.
            self.forgotten = self.forgotten.max(self.revision);
        }
        let revision = self.revision;
        let kept = ids.skip(written.saturating_sub(Self::CAPACITY));
        self.entries.extend(kept.map(|id| (revision, id)));
    }

    /// Forgets the `count` oldest ids the log holds, all of them when it
    /// holds no more.
    fn forget_oldest(&mut self, count: usize) {
        if count == 0 {
            return;
        }
        if count >= self.entries.len() {
            if let Some(&(revision, _)) = self.entries.back() {
                self.forgotten = self.forgotten.max(revision);
            }
            self.entries.clear();
            return;
        }
        for (revision, _) in self.entries.drain(..count) {
            self.forgotten = self.forgotten.max(revision);
        }
    }

    /// The ids the writes after revision `since` changed, repeats included,
    /// or `None` when the log no longer holds all of them.
    pub(crate) fn changed_since(&self, since: u64) -> Option<Vec<u64>> {
        if self.forgotten > since {
            return None;
        }
        Some(
            self.entries
                .iter()
                .filter(|&&(revision, _)| revision > since)
                .map(|&(_, id)| id)
                .collect(),
        )
    }
}

#[cfg(test)]
#[path = "change_log_tests.rs"]
mod tests;
