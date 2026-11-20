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
    /// unknown — as the next revision.
    pub(crate) fn record(&mut self, ids: impl IntoIterator<Item = u64>) {
        self.revision = self.revision.wrapping_add(1);
        for id in ids {
            self.entries.push_back((self.revision, id));
        }
        while self.entries.len() > Self::CAPACITY {
            if let Some((revision, _)) = self.entries.pop_front() {
                self.forgotten = self.forgotten.max(revision);
            }
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
