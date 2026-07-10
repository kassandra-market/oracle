//! Clock access and phase/window guards shared by every instruction processor.

use pinocchio::{error::ProgramError, sysvars::clock::Clock, sysvars::Sysvar, ProgramResult};

use crate::{
    error::KassandraError,
    state::{Oracle, Phase},
};

/// Current on-chain unix timestamp, read from the `Clock` sysvar.
pub fn now() -> Result<i64, ProgramError> {
    Ok(Clock::get()?.unix_timestamp)
}

/// Require that the oracle is currently in phase `p`, else [`KassandraError::WrongPhase`].
pub fn require_phase(o: &Oracle, p: Phase) -> ProgramResult {
    if o.phase != p.as_u8() {
        return Err(KassandraError::WrongPhase.into());
    }
    Ok(())
}

/// Require that the current phase window is still open: `now < phase_ends_at`.
///
/// At or past the deadline (`now >= phase_ends_at`) the window is considered
/// closed and this returns [`KassandraError::WindowClosed`].
pub fn require_before_end(o: &Oracle, now: i64) -> ProgramResult {
    if now >= o.phase_ends_at {
        return Err(KassandraError::WindowClosed.into());
    }
    Ok(())
}

/// Require that the current phase window has elapsed: `now >= phase_ends_at`.
///
/// Before the deadline (`now < phase_ends_at`) this returns
/// [`KassandraError::WindowNotElapsed`].
pub fn require_after_end(o: &Oracle, now: i64) -> ProgramResult {
    if now < o.phase_ends_at {
        return Err(KassandraError::WindowNotElapsed.into());
    }
    Ok(())
}
