//! Host bridge between the evaluator and the session, trace and movie layers.
//!
//! The host owns one optional session manager created from an injected
//! [`BackendFactory`](chronofork_backend_api::BackendFactory). The script
//! library never names a concrete backend; the CLI supplies the factory.
//!
//! Session builtins (`open`, `step`, `fork`, `checkpoint`, `restore`, `select`,
//! `reset`, inspection reads) and trace capture are implemented here. Movie
//! verification uses the same injected factory through the existing
//! `chronofork-sessions` replay path; no movie or trace codec is duplicated.

use std::sync::Arc;

use chronofork_backend_api::{BackendError, BackendFactory};
use chronofork_core::{InputFrame, SessionId, SnapshotId};
use chronofork_sessions::{Movie, MovieError, SessionError, SessionInspection, SessionManager};
use chronofork_trace::{Trace, TraceConfig};

use crate::error::EvalErrorKind;

/// Session-, trace- and movie-facing builtin implementation.
pub(crate) struct Host {
    factory: Arc<dyn BackendFactory>,
    manager: Option<SessionManager>,
    current: Option<SessionId>,
}

impl Host {
    pub(crate) fn new(factory: Arc<dyn BackendFactory>) -> Self {
        Self {
            factory,
            manager: None,
            current: None,
        }
    }

    /// Creates the manager and one root session, returning the session id.
    pub(crate) fn open(&mut self) -> Result<u64, EvalErrorKind> {
        if self.manager.is_some() {
            return Err(EvalErrorKind::AlreadyOpen);
        }
        let mut manager = SessionManager::new(Arc::clone(&self.factory));
        let session = manager.create_session().map_err(map_session_error)?;
        self.current = Some(session);
        self.manager = Some(manager);
        Ok(session.raw())
    }

    fn manager(&self) -> Result<&SessionManager, EvalErrorKind> {
        self.manager.as_ref().ok_or(EvalErrorKind::NotOpen)
    }

    fn manager_mut(&mut self) -> Result<&mut SessionManager, EvalErrorKind> {
        self.manager.as_mut().ok_or(EvalErrorKind::NotOpen)
    }

    fn current_session(&self) -> Result<SessionId, EvalErrorKind> {
        self.current.ok_or(EvalErrorKind::NotOpen)
    }

    fn inspection(&self) -> Result<SessionInspection, EvalErrorKind> {
        let manager = self.manager()?;
        let session = self.current_session()?;
        manager.inspect(session).map_err(map_session_error)
    }

    /// Returns the current tick as a raw count.
    pub(crate) fn tick(&self) -> Result<u64, EvalErrorKind> {
        Ok(self.inspection()?.machine.tick.raw())
    }

    /// Returns the current canonical state hash as lowercase hexadecimal.
    pub(crate) fn state_hash(&self) -> Result<String, EvalErrorKind> {
        Ok(self.inspection()?.machine.state_hash.to_hex())
    }

    /// Reads one named register from the current session.
    pub(crate) fn register(&self, name: &str) -> Result<u64, EvalErrorKind> {
        self.inspection()?
            .machine
            .registers
            .get(name)
            .copied()
            .ok_or_else(|| EvalErrorKind::UnknownRegister(name.to_owned()))
    }

    /// Reads emulated memory through the session debugger path.
    pub(crate) fn read(&self, address: u64, length: usize) -> Result<Vec<u8>, EvalErrorKind> {
        let manager = self.manager()?;
        let session = self.current_session()?;
        manager
            .read_memory(session, address, length)
            .map_err(map_session_error)
    }

    /// Executes one backend step with neutral input and returns the new tick.
    pub(crate) fn step_once(&mut self) -> Result<u64, EvalErrorKind> {
        let session = self.current.ok_or(EvalErrorKind::NotOpen)?;
        let manager = self.manager_mut()?;
        let outcome = manager
            .step(session, InputFrame::NEUTRAL)
            .map_err(map_session_error)?;
        Ok(outcome.tick.raw())
    }

    /// Forks the current session at its current state and selects the child.
    pub(crate) fn fork(&mut self) -> Result<u64, EvalErrorKind> {
        let source = self.current.ok_or(EvalErrorKind::NotOpen)?;
        let manager = self.manager_mut()?;
        let child = manager.fork_current(source).map_err(map_session_error)?;
        self.current = Some(child);
        Ok(child.raw())
    }

    /// Captures a checkpoint of the current session and returns its id.
    pub(crate) fn checkpoint(&mut self) -> Result<u64, EvalErrorKind> {
        let session = self.current.ok_or(EvalErrorKind::NotOpen)?;
        let manager = self.manager_mut()?;
        let snapshot = manager.snapshot(session).map_err(map_session_error)?;
        Ok(snapshot.raw())
    }

    /// Restores a checkpoint of the current session.
    pub(crate) fn restore(&mut self, snapshot: u64) -> Result<(), EvalErrorKind> {
        let session = self.current.ok_or(EvalErrorKind::NotOpen)?;
        let manager = self.manager_mut()?;
        manager
            .restore(session, SnapshotId::from_raw(snapshot))
            .map_err(map_session_error)
    }

    /// Selects another session registered in this manager.
    pub(crate) fn select(&mut self, session: u64) -> Result<(), EvalErrorKind> {
        let manager = self.manager()?;
        let selected = SessionId::from_raw(session);
        manager.inspect(selected).map_err(map_session_error)?;
        self.current = Some(selected);
        Ok(())
    }

    /// Restores the current session's creation state.
    pub(crate) fn reset(&mut self) -> Result<(), EvalErrorKind> {
        let session = self.current.ok_or(EvalErrorKind::NotOpen)?;
        let manager = self.manager_mut()?;
        manager.reset(session).map_err(map_session_error)
    }

    /// Captures a trace from the current session with neutral input.
    pub(crate) fn capture_trace(
        &mut self,
        config: &TraceConfig,
        steps: u64,
    ) -> Result<Trace, EvalErrorKind> {
        let session = self.current.ok_or(EvalErrorKind::NotOpen)?;
        let manager = self.manager_mut()?;
        manager
            .capture_trace(session, config, steps)
            .map_err(map_session_error)
    }

    /// Replays a decoded movie in a fresh session from the injected factory.
    ///
    /// A recorded expectation that diverges is reported as `Ok(false)`, not as
    /// an error. Malformed movies were rejected by the caller's decode step;
    /// import/backend failures remain typed errors.
    pub(crate) fn verify_movie(&self, movie: &Movie) -> Result<bool, EvalErrorKind> {
        let mut manager = SessionManager::new(Arc::clone(&self.factory));
        match movie.verify(&mut manager) {
            Ok(_) => Ok(true),
            Err(MovieError::Divergence { .. } | MovieError::TickMismatch { .. }) => Ok(false),
            Err(error) => Err(EvalErrorKind::MovieReplay(error.to_string())),
        }
    }
}

fn map_session_error(error: SessionError) -> EvalErrorKind {
    match error {
        SessionError::Halted(_) => EvalErrorKind::Halted,
        SessionError::UnknownSession(_) => EvalErrorKind::UnknownSession,
        SessionError::UnknownSnapshot { .. } => EvalErrorKind::UnknownSnapshot,
        SessionError::Backend(BackendError::OutOfBounds {
            address, length, ..
        }) => EvalErrorKind::MemoryOutOfBounds { address, length },
        SessionError::Trace(error) => EvalErrorKind::TraceCapture(error.to_string()),
        other => EvalErrorKind::Session(other.to_string()),
    }
}
