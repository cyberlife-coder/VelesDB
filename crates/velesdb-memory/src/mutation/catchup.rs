use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::embedder::Embedder;
use crate::storage::NativeStore;
use crate::{MemoryError, MemoryService};

use super::journal::{DirtyJournal, RECORD_BYTES};
use super::DirtyKey;

mod facts;

const MAX_BATCH: usize = 4_096;

#[derive(Debug, Clone, Copy)]
pub(crate) struct CatchUpConfig {
    pub(crate) fact_batch: usize,
    pub(crate) replay_batch: usize,
    pub(crate) edge_cap: usize,
}

impl CatchUpConfig {
    pub(crate) fn validated(self) -> Result<Self, MemoryError> {
        let limits = [self.fact_batch, self.replay_batch, self.edge_cap];
        if limits
            .into_iter()
            .any(|value| value == 0 || value > MAX_BATCH)
        {
            return Err(capture("online migration limits must be in 1..=4096"));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BaseCopyProgress {
    pub(crate) facts: u64,
    pub(crate) edge_sets: u64,
    pub(crate) batches: u64,
    pub(crate) start_watermark: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ReplayProgress {
    pub(crate) records: u64,
    pub(crate) dirty_keys: u64,
    pub(crate) distinct_dirty_facts: u64,
    pub(crate) distinct_edge_sources: u64,
    pub(crate) input_watermark: u64,
    pub(crate) output_watermark: u64,
    pub(crate) backlog: u64,
    pub(crate) pending_journal_bytes: u64,
    pub(crate) elapsed: Duration,
    pub(crate) largest_apply_latency: Duration,
}

pub(crate) struct OnlineCatchUp<'a, E: Embedder> {
    source: &'a MemoryService<E, NativeStore>,
    destination: &'a NativeStore,
    target_embedder: &'a dyn Embedder,
    journal: Arc<DirtyJournal>,
    config: CatchUpConfig,
    start_watermark: u64,
    #[cfg(test)]
    fault: std::sync::atomic::AtomicU8,
}

impl<'a, E: Embedder> OnlineCatchUp<'a, E> {
    pub(crate) fn start(
        source: &'a MemoryService<E, NativeStore>,
        destination: &'a NativeStore,
        target_embedder: &'a dyn Embedder,
        journal: Arc<DirtyJournal>,
        config: CatchUpConfig,
    ) -> Result<Self, MemoryError> {
        let config = config.validated()?;
        let start_watermark = journal.last_sequence();
        source.install_mutation_observer(Some(journal.clone()))?;
        Ok(Self {
            source,
            destination,
            target_embedder,
            journal,
            config,
            start_watermark,
            #[cfg(test)]
            fault: std::sync::atomic::AtomicU8::new(0),
        })
    }

    pub(crate) fn copy_base(&self) -> Result<BaseCopyProgress, MemoryError> {
        self.copy_base_inner(&mut |_| Ok(()))
    }

    #[cfg(test)]
    pub(super) fn copy_base_with_page_hook<F>(
        &self,
        mut hook: F,
    ) -> Result<BaseCopyProgress, MemoryError>
    where
        F: FnMut(u64) -> Result<(), MemoryError>,
    {
        self.copy_base_inner(&mut hook)
    }

    fn copy_base_inner(
        &self,
        hook: &mut dyn FnMut(u64) -> Result<(), MemoryError>,
    ) -> Result<BaseCopyProgress, MemoryError> {
        let (facts, batches) = self.copy_facts(hook)?;
        let edge_sets = self.copy_edges()?;
        Ok(BaseCopyProgress {
            facts,
            edge_sets,
            batches,
            start_watermark: self.start_watermark,
        })
    }

    pub(crate) fn catch_up_batch(&self) -> Result<ReplayProgress, MemoryError> {
        self.source.migration_exclusive(|| self.catch_up_locked())
    }

    fn catch_up_locked(&self) -> Result<ReplayProgress, MemoryError> {
        let started = Instant::now();
        let input_watermark = self.journal.last_sequence();
        let acknowledged = self.journal.compacted_through();
        let records = self
            .journal
            .records_after(acknowledged, self.config.replay_batch)?;
        if records.is_empty() {
            return Ok(Self::replay_progress(
                0,
                DirtyCounts::default(),
                input_watermark,
                acknowledged,
                started.elapsed(),
                Duration::ZERO,
            ));
        }
        let dirty: BTreeSet<DirtyKey> = records.iter().map(|record| record.key).collect();
        let mut largest_apply_latency = Duration::ZERO;
        for key in &dirty {
            let apply_started = Instant::now();
            self.sync_key(*key)?;
            largest_apply_latency = largest_apply_latency.max(apply_started.elapsed());
        }
        self.maybe_fail(FaultPoint::BeforeWatermark)?;
        let watermark = records
            .last()
            .map_or(acknowledged, |record| record.sequence);
        self.journal.compact_through(watermark)?;
        self.maybe_fail(FaultPoint::AfterWatermark)?;
        Ok(Self::replay_progress(
            records.len(),
            DirtyCounts::from_keys(&dirty),
            input_watermark,
            watermark,
            started.elapsed(),
            largest_apply_latency,
        ))
    }

    pub(crate) fn finish(self) -> Result<(), MemoryError> {
        self.source.install_mutation_observer(None)
    }

    fn copy_facts(
        &self,
        hook: &mut dyn FnMut(u64) -> Result<(), MemoryError>,
    ) -> Result<(u64, u64), MemoryError> {
        let mut cursor = None;
        let mut facts = 0_u64;
        let mut batches = 0_u64;
        loop {
            let (page, next) = self
                .source
                .migration_store()
                .migration_list(cursor, self.config.fact_batch)?;
            if page.is_empty() {
                break;
            }
            facts::copy_page(self.destination, self.target_embedder, &page)?;
            facts = facts.saturating_add(to_u64(page.len()));
            batches = batches.saturating_add(1);
            hook(batches)?;
            let Some(next) = next else { break };
            cursor = Some(next);
        }
        Ok((facts, batches))
    }

    fn copy_edges(&self) -> Result<u64, MemoryError> {
        let mut cursor = None;
        let mut edge_sets = 0_u64;
        loop {
            let (page, next) = self
                .source
                .migration_store()
                .migration_list(cursor, self.config.fact_batch)?;
            if page.is_empty() {
                break;
            }
            for fact in page {
                self.copy_edge_set(fact.id)?;
                edge_sets = edge_sets.saturating_add(1);
            }
            let Some(next) = next else { break };
            cursor = Some(next);
        }
        Ok(edge_sets)
    }

    fn sync_key(&self, key: DirtyKey) -> Result<(), MemoryError> {
        match key {
            DirtyKey::Fact(id) => {
                facts::sync(
                    self.source.migration_store(),
                    self.destination,
                    self.target_embedder,
                    id,
                )?;
                self.maybe_fail(FaultPoint::AfterFact)
            }
            DirtyKey::OutgoingEdges(id) => {
                self.sync_edges(id)?;
                self.maybe_fail(FaultPoint::AfterEdges)
            }
        }
    }

    fn sync_edges(&self, from: u64) -> Result<(), MemoryError> {
        let source = self.source.migration_store();
        if !facts::sync(source, self.destination, self.target_embedder, from)? {
            return Ok(());
        }
        self.apply_edge_set(from, false)
    }

    fn copy_edge_set(&self, from: u64) -> Result<(), MemoryError> {
        self.apply_edge_set(from, true)
    }

    fn apply_edge_set(&self, from: u64, skip_absent_empty: bool) -> Result<(), MemoryError> {
        let source = self.source.migration_store();
        let edges = source.migration_live_edges(from, self.config.edge_cap)?;
        if !self.ensure_edge_source(from, edges.is_empty(), skip_absent_empty)? {
            return Ok(());
        }
        self.ensure_edge_targets(&edges)?;
        self.destination
            .migration_replace_edges(from, &edges, self.config.edge_cap)
    }

    fn ensure_edge_source(
        &self,
        from: u64,
        edges_empty: bool,
        skip_absent_empty: bool,
    ) -> Result<bool, MemoryError> {
        if self.destination.migration_contains(from)? {
            return Ok(true);
        }
        if skip_absent_empty && edges_empty {
            return Ok(false);
        }
        facts::sync(
            self.source.migration_store(),
            self.destination,
            self.target_embedder,
            from,
        )
    }

    fn ensure_edge_targets(&self, edges: &[velesdb_core::GraphEdge]) -> Result<(), MemoryError> {
        let source = self.source.migration_store();
        for target in edges.iter().map(velesdb_core::GraphEdge::target) {
            if !self.destination.migration_contains(target)? {
                facts::sync(source, self.destination, self.target_embedder, target)?;
            }
        }
        Ok(())
    }

    fn replay_progress(
        records: usize,
        dirty: DirtyCounts,
        input_watermark: u64,
        output_watermark: u64,
        elapsed: Duration,
        largest_apply_latency: Duration,
    ) -> ReplayProgress {
        let backlog = input_watermark.saturating_sub(output_watermark);
        ReplayProgress {
            records: to_u64(records),
            dirty_keys: dirty.facts.saturating_add(dirty.edge_sources),
            distinct_dirty_facts: dirty.facts,
            distinct_edge_sources: dirty.edge_sources,
            input_watermark,
            output_watermark,
            backlog,
            pending_journal_bytes: backlog.saturating_mul(RECORD_BYTES),
            elapsed,
            largest_apply_latency,
        }
    }

    #[cfg(test)]
    pub(super) fn fail_once_at(&self, point: FaultPoint) {
        self.fault
            .store(point as u8, std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    fn maybe_fail(&self, point: FaultPoint) -> Result<(), MemoryError> {
        use std::sync::atomic::Ordering;
        if self
            .fault
            .compare_exchange(point as u8, 0, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return Err(capture(format!("injected catch-up fault at {point:?}")));
        }
        Ok(())
    }

    #[cfg(not(test))]
    #[allow(clippy::unnecessary_wraps, clippy::unused_self)]
    fn maybe_fail(&self, _point: FaultPoint) -> Result<(), MemoryError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct DirtyCounts {
    facts: u64,
    edge_sources: u64,
}

impl DirtyCounts {
    fn from_keys(keys: &BTreeSet<DirtyKey>) -> Self {
        keys.iter().fold(Self::default(), |mut counts, key| {
            match key {
                DirtyKey::Fact(_) => counts.facts = counts.facts.saturating_add(1),
                DirtyKey::OutgoingEdges(_) => {
                    counts.edge_sources = counts.edge_sources.saturating_add(1);
                }
            }
            counts
        })
    }
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub(super) enum FaultPoint {
    AfterFact = 1,
    AfterEdges = 2,
    BeforeWatermark = 3,
    AfterWatermark = 4,
}

fn to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn capture(message: impl Into<String>) -> MemoryError {
    MemoryError::MigrationCapture(message.into())
}
