//! Streaming ingestion entry points of a [`Collection`]: a point is checked as
//! an upserted one is, then queued for the background drain.

use crate::collection::streaming::{BackpressureError, StreamIngester, StreamingConfig};
use crate::collection::types::Collection;
use crate::point::Point;

impl Collection {
    /// Refuses, before anything is queued, a point an upsert would refuse:
    /// the drain writes it later, where a refusal reaches no caller.
    fn validate_streamed(&self, points: &[Point]) -> Result<(), BackpressureError> {
        self.validate_points_for_upsert(points)
            .map_err(|_| BackpressureError::InvalidPoint)
    }

    /// Sends a point into the streaming ingestion channel, once it passes the
    /// checks an upsert runs.
    ///
    /// # Errors
    ///
    /// Returns [`BackpressureError::InvalidPoint`] — nothing queued — for a
    /// point an upsert would refuse ([`Self::validate_points_for_upsert`]
    /// names the refusal, with core's code), else [`BackpressureError`] on
    /// buffer-full, drain-dead or not-configured.
    pub fn stream_insert(&self, point: Point) -> Result<(), BackpressureError> {
        self.validate_streamed(std::slice::from_ref(&point))?;
        let guard = self.streaming.stream_ingester.read();
        match guard.as_ref() {
            Some(ingester) => ingester.try_send(point),
            None => Err(BackpressureError::NotConfigured),
        }
    }

    /// Sends a batch of points into the streaming ingestion channel.
    ///
    /// Acquires the ingester read-lock once for the entire batch, eliminating
    /// per-point lock overhead. Returns the number of points successfully
    /// queued. If the channel fills mid-batch, returns
    /// [`BackpressureError::BufferFull`] (points already sent are still queued).
    ///
    /// # Errors
    ///
    /// Returns [`BackpressureError::InvalidPoint`] — nothing queued — when a
    /// point fails the checks an upsert runs ([`Self::validate_points_for_upsert`]
    /// names it), else [`BackpressureError`] on buffer-full, drain-dead, or
    /// not-configured.
    pub fn stream_insert_batch(&self, points: Vec<Point>) -> Result<usize, BackpressureError> {
        self.validate_streamed(&points)?;
        let guard = self.streaming.stream_ingester.read();
        match guard.as_ref() {
            Some(ingester) => ingester.try_send_batch(points),
            None => Err(BackpressureError::NotConfigured),
        }
    }

    /// Enables streaming ingestion on this collection.
    ///
    /// Creates a [`StreamIngester`] with the given `config` and stores it in
    /// the `stream_ingester` field. Points can then be submitted via
    /// [`stream_insert`](Self::stream_insert).
    ///
    /// Calling this when streaming is already active replaces the existing
    /// ingester (the old drain task is aborted via `Drop`).
    ///
    /// Future: auto-enable from persisted StreamingConfig on open (STREAM-01)
    ///
    /// # Errors
    ///
    /// Returns [`Error::Config`](crate::error::Error::Config) for a config
    /// [`StreamingConfig::validate`] refuses; the current ingester, if any, is
    /// left running.
    pub fn enable_streaming(&self, config: StreamingConfig) -> crate::error::Result<()> {
        config.validate()?;
        let ingester = StreamIngester::new(self.clone(), config);
        *self.streaming.stream_ingester.write() = Some(ingester);
        Ok(())
    }
}
