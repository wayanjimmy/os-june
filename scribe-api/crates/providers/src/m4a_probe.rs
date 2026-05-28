use scribe_domain::{AudioDurationProbe, DomainError};
use std::{io::Cursor, time::Duration};
use thiserror::Error;

pub struct M4aDurationProbe;

impl M4aDurationProbe {
    pub fn probe(bytes: &[u8]) -> Result<Duration, M4aProbeError> {
        // mp4::Mp4Reader requires a Seek + Read source and the total size in
        // bytes — Cursor satisfies the trait bounds and the slice's length is
        // already known.
        let size = bytes.len() as u64;
        let reader = mp4::Mp4Reader::read_header(Cursor::new(bytes), size)?;
        // Duration on Mp4Reader is computed from the mvhd box; convert it to
        // a wall-clock Duration via the moov timescale.
        Ok(reader.duration())
    }
}

impl AudioDurationProbe for M4aDurationProbe {
    fn probe(&self, audio: &[u8]) -> Result<Duration, DomainError> {
        Self::probe(audio).map_err(DomainError::from)
    }
}

#[derive(Debug, Error)]
pub enum M4aProbeError {
    #[error(transparent)]
    Mp4(#[from] mp4::Error),
}

impl From<M4aProbeError> for DomainError {
    fn from(error: M4aProbeError) -> Self {
        Self::InvalidInput {
            reason: error.to_string(),
        }
    }
}
