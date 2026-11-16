//! The state directory: one `<game id>.json` per game, rewritten whole and atomically
//! after every change. The accounts' database lies beside them (`arena::db`). Games the
//! server stops hosting are moved to `expired/`, never deleted, and are still read from
//! there to be looked at.

use serde::Serialize;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

/// The subdirectory of the state directory holding games no longer hosted.
pub const EXPIRED: &str = "expired";

pub struct Store {
    dir: PathBuf,
    /// Held locked while the directory is in use, so two servers never share it.
    _lock: fs::File,
}

impl Store {
    pub fn open(dir: &Path) -> io::Result<Store> {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(dir)?;
        let lock = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join(".lock"))?;
        lock.try_lock().map_err(|e| match e {
            fs::TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::ResourceBusy,
                "another dominion server is using this directory",
            ),
            fs::TryLockError::Error(e) => e,
        })?;
        // Fail now rather than at the first save.
        let probe = dir.join(".probe.tmp");
        fs::write(&probe, b"")?;
        fs::remove_file(&probe)?;
        Ok(Store {
            dir: dir.to_path_buf(),
            _lock: lock,
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Writes `<key>.json`.
    pub fn save(&self, key: &str, value: &impl Serialize) -> io::Result<()> {
        let tmp = self.dir.join(format!(".{key}.json.tmp"));
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let file = options.open(&tmp)?;
        let mut out = BufWriter::new(&file);
        serde_json::to_writer(&mut out, value)?;
        out.flush()?;
        drop(out);
        file.sync_all()?;
        fs::rename(&tmp, self.dir.join(format!("{key}.json")))
    }

    /// Deletes `<key>.json`. Never used for a game: see [`Store::retire`].
    pub fn remove(&self, key: &str) -> io::Result<()> {
        match fs::remove_file(self.dir.join(format!("{key}.json"))) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Where games the server no longer hosts are kept.
    pub fn expired_dir(&self) -> PathBuf {
        self.dir.join(EXPIRED)
    }

    /// Moves `<key>.json` into `expired/`, replacing an older copy there.
    pub fn retire(&self, key: &str) -> io::Result<()> {
        let expired = self.expired_dir();
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&expired)?;
        let name = format!("{key}.json");
        match fs::rename(self.dir.join(&name), expired.join(&name)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}
