use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::{
    assess, phase_for, validate_sample, ControllerConfig, ControllerPhase, ConvergenceSample,
    RECOVER_CUTOVER, RESUME_CATCH_UP,
};
use crate::MemoryError;

const STATE_VERSION: u32 = 1;
const STATE_FILE: &str = "online-migration-controller.json";
const STAGING_FILE: &str = "online-migration-controller.json.tmp";
const MAX_STATE_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct ControllerState {
    pub(super) version: u32,
    pub(super) epoch_id: String,
    pub(super) config: ControllerConfig,
    pub(super) phase: ControllerPhase,
    pub(super) samples: Vec<ConvergenceSample>,
    pub(super) last_observation: Option<ConvergenceSample>,
    pub(super) last_verdict: Option<super::ConvergenceVerdict>,
    pub(super) recovery_action: Option<String>,
    #[serde(default)]
    pub(super) measured_cutover: Option<std::time::Duration>,
}

pub(super) struct StateStore {
    workspace: PathBuf,
    path: PathBuf,
}

impl StateStore {
    pub(super) fn open(
        workspace: &Path,
        epoch_id: &str,
        config: ControllerConfig,
    ) -> Result<(Self, ControllerState, bool), MemoryError> {
        validate_workspace(workspace)?;
        validate_epoch_id(epoch_id)?;
        let store = Self {
            workspace: workspace.to_owned(),
            path: workspace.join(STATE_FILE),
        };
        store.recover_staging()?;
        let resumed = path_exists(&store.path)?;
        let state = if resumed {
            store.load(epoch_id, config)?
        } else {
            let state = ControllerState {
                version: STATE_VERSION,
                epoch_id: epoch_id.to_owned(),
                config,
                phase: ControllerPhase::CatchingUp,
                samples: Vec::with_capacity(config.observation_window),
                last_observation: None,
                last_verdict: None,
                recovery_action: None,
                measured_cutover: None,
            };
            store.save(&state)?;
            state
        };
        Ok((store, state, resumed))
    }

    pub(super) fn save(&self, state: &ControllerState) -> Result<(), MemoryError> {
        self.recover_staging()?;
        let bytes = serde_json::to_vec_pretty(state)
            .map_err(|err| capture(format!("cannot encode controller state: {err}")))?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(capture("controller state exceeds 64 KiB safety limit"));
        }
        let staging = self.workspace.join(STAGING_FILE);
        write_synced(&staging, &bytes)?;
        promote(&staging, &self.path)
            .map_err(|err| capture(format!("cannot publish controller state: {err}")))?;
        sync_directory(&self.workspace)
            .map_err(|err| capture(format!("cannot sync controller directory: {err}")))
    }

    fn load(
        &self,
        epoch_id: &str,
        config: ControllerConfig,
    ) -> Result<ControllerState, MemoryError> {
        let bytes = read_state_bytes(&self.path)?;
        let state: ControllerState = serde_json::from_slice(&bytes)
            .map_err(|err| capture(format!("invalid controller state: {err}")))?;
        validate_loaded(&state, epoch_id, config)?;
        Ok(state)
    }

    fn recover_staging(&self) -> Result<(), MemoryError> {
        let staging = self.workspace.join(STAGING_FILE);
        if !path_exists(&staging)? {
            return Ok(());
        }
        validate_regular_file(&staging)?;
        std::fs::remove_file(&staging)
            .map_err(|err| capture(format!("cannot remove controller staging file: {err}")))
    }
}

fn read_state_bytes(path: &Path) -> Result<Vec<u8>, MemoryError> {
    validate_regular_file(path)?;
    let mut file =
        File::open(path).map_err(|err| capture(format!("cannot open controller state: {err}")))?;
    let length = file
        .metadata()
        .map_err(|err| capture(format!("cannot size controller state: {err}")))?
        .len();
    if length > MAX_STATE_BYTES {
        return Err(capture("controller state exceeds 64 KiB safety limit"));
    }
    let capacity = usize::try_from(length)
        .map_err(|_| capture("controller state length does not fit memory"))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.read_to_end(&mut bytes)
        .map_err(|err| capture(format!("cannot read controller state: {err}")))?;
    Ok(bytes)
}

fn validate_loaded(
    state: &ControllerState,
    epoch_id: &str,
    config: ControllerConfig,
) -> Result<(), MemoryError> {
    validate_header(state, epoch_id, config)?;
    validate_samples(&state.samples)?;
    if let Some(sample) = state.last_observation.as_ref() {
        validate_sample(None, sample)?;
    }
    validate_audit_fields(state)?;
    validate_phase(state)
}

fn validate_header(
    state: &ControllerState,
    epoch_id: &str,
    config: ControllerConfig,
) -> Result<(), MemoryError> {
    if state.version != STATE_VERSION {
        return Err(capture(format!(
            "unsupported controller state version {}",
            state.version
        )));
    }
    if state.epoch_id != epoch_id {
        return Err(capture("controller epoch ownership mismatch"));
    }
    if state.config != config || state.samples.len() > config.observation_window {
        return Err(capture("controller configuration mismatch"));
    }
    Ok(())
}

fn validate_audit_fields(state: &ControllerState) -> Result<(), MemoryError> {
    if state.last_observation.is_some() != state.last_verdict.is_some() {
        return Err(capture("incomplete durable controller audit state"));
    }
    if let Some(last) = state.samples.last() {
        if state.last_observation.as_ref() != Some(last)
            || state.last_verdict != Some(assess(&state.samples, state.config).verdict)
        {
            return Err(capture(
                "controller audit state disagrees with measured window",
            ));
        }
    }
    Ok(())
}

fn validate_samples(samples: &[ConvergenceSample]) -> Result<(), MemoryError> {
    let mut previous = None;
    for sample in samples {
        validate_sample(previous, sample)?;
        previous = Some(sample);
    }
    Ok(())
}

fn validate_phase(state: &ControllerState) -> Result<(), MemoryError> {
    if state.phase != ControllerPhase::Activated && state.measured_cutover.is_some() {
        return Err(capture("cutover duration exists before activation"));
    }
    match state.phase {
        ControllerPhase::CatchingUp
        | ControllerPhase::CutoverReady
        | ControllerPhase::NonConverging => validate_observation_phase(state),
        ControllerPhase::Quiescing { deadline } => validate_quiescing(state, deadline),
        ControllerPhase::Activated => validate_activated(state),
        ControllerPhase::Cancelled => validate_recovery(state, &[None]),
    }
}

fn validate_observation_phase(state: &ControllerState) -> Result<(), MemoryError> {
    if state.samples.is_empty() && state.phase != ControllerPhase::CatchingUp {
        return Err(capture("controller phase requires a measured window"));
    }
    let resuming = state.phase == ControllerPhase::CatchingUp
        && state.recovery_action.as_deref() == Some(RESUME_CATCH_UP);
    if !resuming
        && !state.samples.is_empty()
        && phase_for(assess(&state.samples, state.config).verdict) != state.phase
    {
        return Err(capture("controller phase disagrees with measured window"));
    }
    let allowed = if state.phase == ControllerPhase::CatchingUp {
        [None, Some(RESUME_CATCH_UP)]
    } else {
        [None, None]
    };
    validate_recovery(state, &allowed)
}

fn validate_quiescing(
    state: &ControllerState,
    deadline: std::time::Duration,
) -> Result<(), MemoryError> {
    if !has_cutover_ready_window(state)
        || state
            .samples
            .last()
            .is_some_and(|sample| deadline < sample.observed_at)
    {
        return Err(capture("invalid durable quiescing state"));
    }
    validate_recovery(state, &[None, Some(RECOVER_CUTOVER)])
}

fn validate_activated(state: &ControllerState) -> Result<(), MemoryError> {
    if !has_cutover_ready_window(state)
        || state
            .measured_cutover
            .is_none_or(|elapsed| elapsed > state.config.pause_budget)
    {
        return Err(capture("invalid durable activated state"));
    }
    validate_recovery(state, &[None, Some(RECOVER_CUTOVER)])
}

fn has_cutover_ready_window(state: &ControllerState) -> bool {
    !state.samples.is_empty()
        && phase_for(assess(&state.samples, state.config).verdict) == ControllerPhase::CutoverReady
}

fn validate_recovery(state: &ControllerState, allowed: &[Option<&str>]) -> Result<(), MemoryError> {
    if allowed.contains(&state.recovery_action.as_deref()) {
        Ok(())
    } else {
        Err(capture("invalid durable controller recovery action"))
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), MemoryError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|err| capture(format!("cannot create controller staging file: {err}")))?;
    file.write_all(bytes)
        .and_then(|()| file.flush())
        .and_then(|()| file.sync_all())
        .map_err(|err| capture(format!("cannot sync controller state: {err}")))
}

fn validate_workspace(workspace: &Path) -> Result<(), MemoryError> {
    let metadata = std::fs::symlink_metadata(workspace)
        .map_err(|err| capture(format!("cannot inspect controller workspace: {err}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(capture("controller workspace must be a real directory"));
    }
    Ok(())
}

fn validate_regular_file(path: &Path) -> Result<(), MemoryError> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|err| capture(format!("cannot inspect controller file: {err}")))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(capture("controller path must be a regular file"));
    }
    Ok(())
}

fn path_exists(path: &Path) -> Result<bool, MemoryError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(capture(format!(
            "cannot inspect {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(unix)]
fn promote(staging: &Path, final_path: &Path) -> std::io::Result<()> {
    std::fs::rename(staging, final_path)
}

#[cfg(windows)]
fn promote(staging: &Path, final_path: &Path) -> std::io::Result<()> {
    atomicwrites::replace_atomic(staging, final_path)
}

#[cfg(not(any(unix, windows)))]
fn promote(_staging: &Path, _final_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "controller state replacement unsupported",
    ))
}

#[cfg(unix)]
fn sync_directory(workspace: &Path) -> std::io::Result<()> {
    File::open(workspace)?.sync_all()
}

#[cfg(any(windows, not(any(unix, windows))))]
fn sync_directory(_workspace: &Path) -> std::io::Result<()> {
    Ok(())
}

fn validate_epoch_id(epoch_id: &str) -> Result<(), MemoryError> {
    if epoch_id.len() != 32 || !epoch_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(capture("epoch id must be 32 hexadecimal characters"));
    }
    Ok(())
}

fn capture(message: impl Into<String>) -> MemoryError {
    MemoryError::MigrationCapture(message.into())
}
