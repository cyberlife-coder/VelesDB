use std::time::Duration;

use crate::MemoryError;

use super::catchup::ReplayProgress;

mod state;

use state::{ControllerState, StateStore};

const MAX_WINDOW: usize = 64;
const MAX_BUDGET: Duration = Duration::from_secs(24 * 60 * 60);
const RESUME_CATCH_UP: &str = "reopen source and resume catch-up";
const RECOVER_CUTOVER: &str = "complete or recover cutover before serving traffic";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ControllerConfig {
    pub(crate) observation_window: usize,
    pub(crate) pause_budget: Duration,
    pub(crate) verification_reserve: Duration,
}

impl ControllerConfig {
    fn validate(self) -> Result<Self, MemoryError> {
        if !(2..=MAX_WINDOW).contains(&self.observation_window) {
            return Err(capture("controller observation window must be in 2..=64"));
        }
        if self.pause_budget.is_zero() || self.pause_budget > MAX_BUDGET {
            return Err(capture("controller pause budget must be in 1ns..=24h"));
        }
        if self.verification_reserve > self.pause_budget {
            return Err(capture("verification reserve exceeds pause budget"));
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ConvergenceSample {
    pub(crate) observed_at: Duration,
    pub(crate) input_watermark: u64,
    pub(crate) output_watermark: u64,
    pub(crate) distinct_dirty_facts: u64,
    pub(crate) distinct_edge_sources: u64,
    pub(crate) pending_journal_bytes: u64,
    pub(crate) replay_elapsed: Duration,
    pub(crate) largest_apply_latency: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConvergenceMetrics {
    pub(crate) input_watermark: u64,
    pub(crate) output_watermark: u64,
    pub(crate) backlog_records: u64,
    pub(crate) backlog_grew: bool,
    pub(crate) distinct_dirty_facts: u64,
    pub(crate) distinct_edge_sources: u64,
    pub(crate) pending_journal_bytes: u64,
    pub(crate) arrival_rate: MeasuredRate,
    pub(crate) replay_rate: MeasuredRate,
    pub(crate) window_elapsed: Duration,
    pub(crate) replay_elapsed: Duration,
    pub(crate) largest_apply_latency: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MeasuredRate {
    pub(crate) records: u64,
    pub(crate) elapsed: Duration,
}

impl ConvergenceSample {
    pub(crate) fn from_replay(observed_at: Duration, progress: ReplayProgress) -> Self {
        Self {
            observed_at,
            input_watermark: progress.input_watermark,
            output_watermark: progress.output_watermark,
            distinct_dirty_facts: progress.distinct_dirty_facts,
            distinct_edge_sources: progress.distinct_edge_sources,
            pending_journal_bytes: progress.pending_journal_bytes,
            replay_elapsed: progress.elapsed,
            largest_apply_latency: progress.largest_apply_latency,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum ConvergenceVerdict {
    CatchingUp,
    CutoverReady,
    NonConverging,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConvergenceObservation {
    pub(crate) metrics: ConvergenceMetrics,
    pub(crate) estimated_pause: Option<Duration>,
    pub(crate) verdict: ConvergenceVerdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum ControllerPhase {
    CatchingUp,
    CutoverReady,
    NonConverging,
    Quiescing { deadline: Duration },
    Activated,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CancellationPermit {
    epoch_id: String,
}

impl CancellationPermit {
    pub(crate) fn epoch_id(&self) -> &str {
        &self.epoch_id
    }
}

pub(crate) struct ConvergenceController {
    config: ControllerConfig,
    store: StateStore,
    state: ControllerState,
}

impl ConvergenceController {
    pub(crate) fn open(
        workspace: &std::path::Path,
        epoch_id: &str,
        config: ControllerConfig,
    ) -> Result<Self, MemoryError> {
        let config = config.validate()?;
        let (store, mut state, resumed) = StateStore::open(workspace, epoch_id, config)?;
        if resumed && prepare_resumed_state(&mut state) {
            store.save(&state)?;
        }
        Ok(Self {
            config,
            store,
            state,
        })
    }

    pub(crate) fn observe(
        &mut self,
        sample: ConvergenceSample,
    ) -> Result<ConvergenceObservation, MemoryError> {
        ensure_observable(self.state.phase)?;
        validate_sample(self.state.samples.last(), &sample)?;
        let mut next = self.state.clone();
        next.samples.push(sample);
        if next.samples.len() > self.config.observation_window {
            next.samples.remove(0);
        }
        let observation = assess(&next.samples, self.config);
        next.phase = phase_for(observation.verdict);
        next.last_observation = Some(sample);
        next.last_verdict = Some(observation.verdict);
        next.recovery_action = None;
        self.replace_state(next)?;
        Ok(observation)
    }

    pub(crate) fn begin_quiescing(&mut self, now: Duration) -> Result<(), MemoryError> {
        if self.state.phase != ControllerPhase::CutoverReady {
            return Err(capture("cutover is not ready for quiescing"));
        }
        ensure_clock_after_sample(&self.state, now)?;
        let deadline = now
            .checked_add(self.config.pause_budget)
            .ok_or_else(|| capture("cutover deadline overflow"))?;
        let mut next = self.state.clone();
        next.phase = ControllerPhase::Quiescing { deadline };
        self.replace_state(next)
    }

    pub(crate) fn activate(&mut self, now: Duration) -> Result<(), MemoryError> {
        if self.state.recovery_action.as_deref() == Some(RECOVER_CUTOVER) {
            return Err(capture("cutover recovery is required after restart"));
        }
        let ControllerPhase::Quiescing { deadline } = self.state.phase else {
            return Err(capture("migration is not quiescing"));
        };
        ensure_activation_time(now, deadline, self.config.pause_budget)?;
        if now > deadline {
            let mut next = self.state.clone();
            next.phase = ControllerPhase::CatchingUp;
            next.recovery_action = Some(RESUME_CATCH_UP.to_owned());
            self.replace_state(next)?;
            return Err(capture("cutover deadline expired"));
        }
        let mut next = self.state.clone();
        next.phase = ControllerPhase::Activated;
        next.recovery_action = None;
        self.replace_state(next)
    }

    pub(crate) fn cancel(
        &mut self,
        source_authoritative: bool,
        epoch_id: &str,
    ) -> Result<CancellationPermit, MemoryError> {
        if epoch_id != self.state.epoch_id {
            return Err(capture("controller epoch ownership mismatch"));
        }
        if matches!(
            self.state.phase,
            ControllerPhase::Quiescing { .. } | ControllerPhase::Activated
        ) {
            let mut next = self.state.clone();
            next.recovery_action = Some(RECOVER_CUTOVER.to_owned());
            self.replace_state(next)?;
            return Err(capture("cutover recovery is required; rollback is unsafe"));
        }
        if !source_authoritative {
            return Err(capture("source is not authoritative; cancellation refused"));
        }
        let mut next = self.state.clone();
        next.phase = ControllerPhase::Cancelled;
        next.recovery_action = None;
        self.replace_state(next)?;
        Ok(CancellationPermit {
            epoch_id: epoch_id.to_owned(),
        })
    }

    pub(crate) fn phase(&self) -> ControllerPhase {
        self.state.phase
    }

    pub(crate) fn recovery_action(&self) -> Option<&str> {
        self.state.recovery_action.as_deref()
    }

    fn replace_state(&mut self, next: ControllerState) -> Result<(), MemoryError> {
        self.store.save(&next)?;
        self.state = next;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn retained_samples(&self) -> usize {
        self.state.samples.len()
    }

    #[cfg(test)]
    pub(super) fn last_observation(&self) -> Option<ConvergenceSample> {
        self.state.last_observation
    }
}

fn prepare_resumed_state(state: &mut ControllerState) -> bool {
    match state.phase {
        ControllerPhase::CatchingUp
        | ControllerPhase::CutoverReady
        | ControllerPhase::NonConverging => {
            state.samples.clear();
            state.phase = ControllerPhase::CatchingUp;
            state.recovery_action = Some(RESUME_CATCH_UP.to_owned());
            true
        }
        ControllerPhase::Quiescing { .. } | ControllerPhase::Activated => {
            state.recovery_action = Some(RECOVER_CUTOVER.to_owned());
            true
        }
        ControllerPhase::Cancelled => false,
    }
}

fn validate_sample(
    previous: Option<&ConvergenceSample>,
    sample: &ConvergenceSample,
) -> Result<(), MemoryError> {
    if sample.output_watermark > sample.input_watermark {
        return Err(capture("output watermark exceeds input watermark"));
    }
    if sample.largest_apply_latency > sample.replay_elapsed {
        return Err(capture("largest apply latency exceeds replay elapsed time"));
    }
    if let Some(previous) = previous {
        if sample.observed_at <= previous.observed_at {
            return Err(capture("controller observations must use monotonic time"));
        }
        if sample.input_watermark < previous.input_watermark
            || sample.output_watermark < previous.output_watermark
        {
            return Err(capture("controller watermarks must be monotonic"));
        }
    }
    Ok(())
}

fn assess(samples: &[ConvergenceSample], config: ControllerConfig) -> ConvergenceObservation {
    let metrics = metrics(samples);
    if samples.len() < config.observation_window {
        return observation(metrics, None, ConvergenceVerdict::CatchingUp);
    }
    let estimate = pause_estimate(metrics, config.verification_reserve);
    let verdict = verdict(metrics, estimate, config.pause_budget);
    observation(metrics, estimate, verdict)
}

fn metrics(samples: &[ConvergenceSample]) -> ConvergenceMetrics {
    let first = &samples[0];
    let last = &samples[samples.len() - 1];
    ConvergenceMetrics {
        input_watermark: last.input_watermark,
        output_watermark: last.output_watermark,
        backlog_records: last.input_watermark.saturating_sub(last.output_watermark),
        backlog_grew: last.input_watermark.saturating_sub(last.output_watermark)
            > first.input_watermark.saturating_sub(first.output_watermark),
        distinct_dirty_facts: last.distinct_dirty_facts,
        distinct_edge_sources: last.distinct_edge_sources,
        pending_journal_bytes: last.pending_journal_bytes,
        arrival_rate: MeasuredRate {
            records: last.input_watermark.saturating_sub(first.input_watermark),
            elapsed: last.observed_at.saturating_sub(first.observed_at),
        },
        replay_rate: MeasuredRate {
            records: last.output_watermark.saturating_sub(first.output_watermark),
            elapsed: last.observed_at.saturating_sub(first.observed_at),
        },
        window_elapsed: last.observed_at.saturating_sub(first.observed_at),
        replay_elapsed: last.replay_elapsed,
        largest_apply_latency: samples
            .iter()
            .map(|sample| sample.largest_apply_latency)
            .max()
            .unwrap_or_default(),
    }
}

fn pause_estimate(metrics: ConvergenceMetrics, reserve: Duration) -> Option<Duration> {
    if metrics.replay_rate.records == 0 {
        return None;
    }
    if metrics.backlog_records == 0 {
        return metrics.largest_apply_latency.checked_add(reserve);
    }
    let net_replay = metrics
        .replay_rate
        .records
        .checked_sub(metrics.arrival_rate.records)?;
    if net_replay == 0 || metrics.window_elapsed.is_zero() {
        return None;
    }
    let drain = ceil_duration_product(metrics.window_elapsed, metrics.backlog_records, net_replay);
    drain
        .checked_add(metrics.largest_apply_latency)?
        .checked_add(reserve)
}

fn verdict(
    metrics: ConvergenceMetrics,
    estimate: Option<Duration>,
    budget: Duration,
) -> ConvergenceVerdict {
    if metrics.backlog_grew
        || (metrics.arrival_rate.records > 0
            && metrics.arrival_rate.records >= metrics.replay_rate.records)
    {
        return ConvergenceVerdict::NonConverging;
    }
    match estimate {
        Some(duration) if duration <= budget => ConvergenceVerdict::CutoverReady,
        _ => ConvergenceVerdict::CatchingUp,
    }
}

fn ceil_duration_product(duration: Duration, count: u64, divisor: u64) -> Duration {
    let numerator = duration.as_nanos().saturating_mul(u128::from(count));
    let rounded =
        numerator.saturating_add(u128::from(divisor).saturating_sub(1)) / u128::from(divisor);
    Duration::from_nanos(u64::try_from(rounded).unwrap_or(u64::MAX))
}

fn observation(
    metrics: ConvergenceMetrics,
    estimated_pause: Option<Duration>,
    verdict: ConvergenceVerdict,
) -> ConvergenceObservation {
    ConvergenceObservation {
        metrics,
        estimated_pause,
        verdict,
    }
}

fn phase_for(verdict: ConvergenceVerdict) -> ControllerPhase {
    match verdict {
        ConvergenceVerdict::CatchingUp => ControllerPhase::CatchingUp,
        ConvergenceVerdict::CutoverReady => ControllerPhase::CutoverReady,
        ConvergenceVerdict::NonConverging => ControllerPhase::NonConverging,
    }
}

fn ensure_observable(phase: ControllerPhase) -> Result<(), MemoryError> {
    match phase {
        ControllerPhase::CatchingUp
        | ControllerPhase::CutoverReady
        | ControllerPhase::NonConverging => Ok(()),
        _ => Err(capture("controller phase does not accept observations")),
    }
}

fn ensure_clock_after_sample(state: &ControllerState, now: Duration) -> Result<(), MemoryError> {
    if state
        .samples
        .last()
        .is_some_and(|sample| now < sample.observed_at)
    {
        return Err(capture("cutover clock predates the latest observation"));
    }
    Ok(())
}

fn ensure_activation_time(
    now: Duration,
    deadline: Duration,
    budget: Duration,
) -> Result<(), MemoryError> {
    let started = deadline
        .checked_sub(budget)
        .ok_or_else(|| capture("invalid cutover deadline"))?;
    if now < started {
        return Err(capture("cutover activation time is not monotonic"));
    }
    Ok(())
}

fn capture(message: impl Into<String>) -> MemoryError {
    MemoryError::MigrationCapture(message.into())
}
