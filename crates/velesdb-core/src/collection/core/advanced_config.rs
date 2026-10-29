//! Post-creation advanced configuration: `pq_rescore_oversampling` and the
//! two streaming components, `deferred_indexing` and `async_index_builder`.
//!
//! `pq_rescore_oversampling` is read from the config on every search, so a
//! change is live at once. The streaming components are objects built from
//! the config, and only one direction of change can be made live safely:
//!
//! - **installing** one into a collection that runs none — nothing holds the
//!   empty slot, so a writer either saw no component (and indexed
//!   synchronously, as it always did) or sees the new one;
//! - **replacing or removing** a live one is never done here. A write loads
//!   the component and uses it with no lock held across both steps, so a
//!   writer may still push into the old one after a swap, and what it pushed
//!   would be stranded where neither search nor flush looks. Such a change is
//!   persisted and takes effect at the next open, which builds the
//!   components before anyone holds them; [`Collection::pending_reopen`]
//!   reports it until then.

use std::sync::Arc;

use arc_swap::ArcSwapOption;

use crate::collection::streaming::{AsyncIndexBuilderConfig, PendingReopen};
use crate::collection::types::Collection;
use crate::error::Result;

impl Collection {
    /// Applies post-creation overrides to the advanced configuration
    /// fields and persists the updated `config.json` atomically.
    ///
    /// Used by the server `POST /collections` handler to wire
    /// `pq_rescore_oversampling`, `deferred_indexing`, and
    /// `async_index_builder` from the REST payload after the collection
    /// has been created with its base options (HNSW, storage mode), and by
    /// the bindings' `apply_advanced_config`.
    ///
    /// The `Option<Option<T>>` pattern encodes three states:
    /// `None` (leave unchanged), `Some(None)` (clear the field), and
    /// `Some(Some(v))` (set the field to `v`). A clippy allow is
    /// applied locally because that is exactly what we need here.
    ///
    /// When it takes effect is the module's contract: at once for
    /// `pq_rescore_oversampling` and for a streaming component the
    /// collection does not run yet; at the next open for a change to one it
    /// runs, which [`Self::pending_reopen`] then names.
    ///
    /// # Errors
    ///
    /// Returns an error if the updated config cannot be written to disk.
    #[allow(clippy::option_option)]
    pub(crate) fn apply_advanced_config(
        &self,
        pq_rescore_oversampling: Option<Option<u32>>,
        #[cfg(feature = "persistence")] deferred_indexing: Option<
            Option<crate::collection::streaming::DeferredIndexerConfig>,
        >,
        async_index_builder: Option<Option<AsyncIndexBuilderConfig>>,
    ) -> Result<()> {
        // The config write guard also serializes the installs below: an
        // install is check-then-store, and no other path writes a slot once
        // the collection is assembled.
        let mut config = self.storage.config.write();
        if let Some(rescore) = pq_rescore_oversampling {
            config.pq_rescore_oversampling = rescore;
        }
        #[cfg(feature = "persistence")]
        if let Some(deferred) = deferred_indexing {
            config.deferred_indexing = deferred;
            install_into_empty_slot(
                &self.streaming.deferred_indexer,
                Self::build_deferred_indexer(&config),
            );
        }
        if let Some(aib) = async_index_builder {
            config.async_index_builder = aib;
            install_into_empty_slot(
                &self.streaming.async_index_builder,
                Self::build_async_index_builder(&config),
            );
        }
        drop(config);
        self.save_config()
    }

    /// Lists the streaming settings whose persisted value is not the one in
    /// force, and will be after the next open. Empty when every setting is
    /// live.
    ///
    /// Compares what an open would build from `config.json` with what the
    /// collection runs, so it stays true however the two came apart.
    #[must_use]
    pub(crate) fn pending_reopen(&self) -> Vec<PendingReopen> {
        let config = self.storage.config.read();
        #[cfg(feature = "persistence")]
        let deferred_pending = {
            let wanted = config.deferred_indexing.as_ref().filter(|c| c.enabled);
            let live = self.streaming.deferred_indexer.load();
            wanted != live.as_ref().map(|di| di.config())
        };
        let builder_pending = {
            let live = self.streaming.async_index_builder.load();
            config.async_index_builder.as_ref() != live.as_ref().map(|aib| aib.config())
        };
        drop(config);

        let mut pending = Vec::new();
        #[cfg(feature = "persistence")]
        if deferred_pending {
            pending.push(PendingReopen::DeferredIndexing);
        }
        if builder_pending {
            pending.push(PendingReopen::AsyncIndexBuilder);
        }
        pending
    }

    /// The deferred indexer in force, if any (tests; the write and search
    /// paths `load()` the slot without an `Arc` clone).
    #[cfg(all(test, feature = "persistence"))]
    #[must_use]
    pub(crate) fn deferred_indexer(
        &self,
    ) -> Option<Arc<crate::collection::streaming::DeferredIndexer>> {
        self.streaming.deferred_indexer.load_full()
    }

    /// The async index builder in force, if any (tests; see above).
    #[cfg(test)]
    #[must_use]
    pub(crate) fn async_index_builder(
        &self,
    ) -> Option<Arc<crate::collection::streaming::AsyncIndexBuilder>> {
        self.streaming.async_index_builder.load_full()
    }
}

/// Stores `built` into `slot` only when the slot is empty — the one change
/// the module doc shows to be safe on a shared collection. A slot already
/// holding a component keeps it; the persisted config then differs from it,
/// which is what `pending_reopen` reports.
fn install_into_empty_slot<T>(slot: &ArcSwapOption<T>, built: Option<Arc<T>>) {
    if built.is_some() && slot.load().is_none() {
        slot.store(built);
    }
}
