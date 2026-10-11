//! Small readers used after a codec has validated its complete record length.
use anyhow::{ensure, Result};

pub(crate) fn word(body: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]])
}

pub(crate) fn spawn_id(value: u32) -> Result<u16> {
    let id = u16::try_from(value)?;
    ensure!(id != 0, "invalid zero spawn ID");
    Ok(id)
}
