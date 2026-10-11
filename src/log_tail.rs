use crate::Result;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

/// Which file this is, so a rotated log starts again from the top. Unix: device and inode. Elsewhere
/// the creation time stands in (a rotated log is a new file with a new creation time).
#[cfg(unix)]
fn identity(meta: &std::fs::Metadata) -> (u64, u64) {
    (meta.dev(), meta.ino())
}
#[cfg(not(unix))]
fn identity(meta: &std::fs::Metadata) -> (u64, u64) {
    let born = meta.created().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos() as u64).unwrap_or(0);
    (born, 0)
}

#[derive(Default)]
pub struct LogTail {
    identity: Option<(u64, u64)>,
    offset: u64,
    pending: Vec<u8>,
}
impl LogTail {
    pub fn read_new(&mut self, path: &str, max_bytes: usize) -> Result<(Vec<String>, Vec<String>)> {
        let mut file=File::open(path).map_err(|e|
            format!("cannot read klippy.log at {path}: {e}; set klippy_log_path to the local log and grant read permission"))?;
        let meta = file
            .metadata()
            .map_err(|e| format!("cannot inspect klippy.log: {e}; check the log mount"))?;
        let identity = identity(&meta);
        let mut notices = Vec::new();
        if self.identity.is_none() {
            self.identity = Some(identity);
            self.offset = meta.len();
            return Ok((Vec::new(),vec!["Log capture starts at the current end of klippy.log; older log lines were not imported".into()]));
        }
        if self.identity != Some(identity) || meta.len() < self.offset {
            self.offset = 0;
            self.pending.clear();
            self.identity = Some(identity);
            notices.push("klippy.log rotated or was truncated; reading the new file, but unread lines in the old file may be unavailable".into());
        }
        file.seek(SeekFrom::Start(self.offset))
            .map_err(|e| format!("cannot seek klippy.log: {e}; check the log file"))?;
        let mut bytes = Vec::new();
        file.take(max_bytes as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| {
                format!("cannot read klippy.log tail: {e}; check disk and mount health")
            })?;
        self.offset += bytes.len() as u64;
        self.pending.extend(bytes);
        let mut lines = Vec::new();
        if let Some(last) = self.pending.iter().rposition(|b| *b == b'\n') {
            let complete = self.pending.drain(..=last).collect::<Vec<_>>();
            for raw in complete.split(|b| *b == b'\n') {
                let text=std::str::from_utf8(raw).map_err(|_|"klippy.log contains invalid UTF-8; repair or rotate the log before continuing capture".to_owned())?;
                if text.contains("position skew after probe:") {
                    lines.push(text.trim_end_matches('\r').to_owned());
                }
            }
        }
        if self.pending.len() > max_bytes {
            self.pending.clear();
            notices.push("An oversized unfinished log line was discarded; inspect klippy.log for malformed output".into());
        }
        if meta.len() > self.offset {
            notices.push(format!(
                "Log capture is catching up: {} bytes remain",
                meta.len() - self.offset
            ));
        }
        Ok((lines, notices))
    }
}
