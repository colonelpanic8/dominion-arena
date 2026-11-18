//! The state directory: one `<game id>.json` per game, rewritten whole and atomically
//! after every change, and beside it the game's recorded event stream,
//! `<game id>.events.<jsonl|cbor>` ([`dominion_agent::recording`]), appended to. The
//! accounts' database lies beside them (`arena::db`). Games the server stops hosting are
//! moved to `expired/`, never deleted, and are still read from there to be looked at.

use dominion_agent::recording::{self, Encoding};
use dominion_protocol::events::Event;
use serde::Serialize;
use std::fs;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
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
        let file = private_file(&tmp)?;
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

    /// Moves `<key>.json` and its event stream into `expired/`, replacing older copies
    /// there.
    pub fn retire(&self, key: &str) -> io::Result<()> {
        let expired = self.expired_dir();
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&expired)?;
        let names = Encoding::ALL.map(|e| format!("{key}{}", e.suffix()));
        for name in [format!("{key}.json")].iter().chain(&names) {
            match fs::rename(self.dir.join(name), expired.join(name)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        Ok(())
    }

    /// Game `key`'s event stream, here or in `expired/`, with its encoding: the first
    /// found.
    pub fn stream_file(&self, key: &str) -> Option<(PathBuf, Encoding)> {
        [self.dir.clone(), self.expired_dir()]
            .iter()
            .flat_map(|dir| Encoding::ALL.map(|e| (dir.join(format!("{key}{}", e.suffix())), e)))
            .find(|(path, _)| path.exists())
    }

    /// Writes `events` to game `key`'s stream in `encoding`, after its first `len` bytes,
    /// which hold the events before them; whatever followed those (a write cut short) is
    /// dropped. A stream that begins at event 0 replaces the file, atomically. Synced
    /// before it returns. The stream's new length.
    pub fn write_stream(
        &self,
        key: &str,
        encoding: Encoding,
        len: u64,
        events: &[Event],
    ) -> io::Result<u64> {
        let path = self.dir.join(format!("{key}{}", encoding.suffix()));
        if events.first().is_some_and(|e| e.seq == 0) {
            let tmp = self.dir.join(format!(".{key}{}.tmp", encoding.suffix()));
            let file = private_file(&tmp)?;
            let mut out = BufWriter::new(&file);
            encoding.write(&mut out, &recording::header(key))?;
            for event in events {
                encoding.write(&mut out, event)?;
            }
            out.flush()?;
            drop(out);
            file.sync_all()?;
            fs::rename(&tmp, &path)?;
            return file.metadata().map(|m| m.len());
        }
        let mut file = fs::OpenOptions::new().write(true).open(&path)?;
        file.set_len(len)?;
        file.seek(SeekFrom::Start(len))?;
        let mut out = BufWriter::new(&file);
        for event in events {
            encoding.write(&mut out, event)?;
        }
        out.flush()?;
        drop(out);
        file.sync_data()?;
        file.stream_position()
    }
}

fn private_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}
