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
    /// Test hooks: how many of the next snapshot and stream writes fail.
    #[cfg(test)]
    pub(crate) fail_saves: std::sync::atomic::AtomicU32,
    #[cfg(test)]
    pub(crate) fail_streams: std::sync::atomic::AtomicU32,
}

/// Takes one of the failures a test hook has waiting.
#[cfg(test)]
fn injected(count: &std::sync::atomic::AtomicU32) -> io::Result<()> {
    use std::sync::atomic::Ordering;
    match count.try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1)) {
        Ok(_) => Err(io::Error::other("a write failure injected for a test")),
        Err(_) => Ok(()),
    }
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
            #[cfg(test)]
            fail_saves: Default::default(),
            #[cfg(test)]
            fail_streams: Default::default(),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Writes `<key>.json`.
    pub fn save(&self, key: &str, value: &impl Serialize) -> io::Result<()> {
        self.save_bytes(key, &serde_json::to_vec(value)?)
    }

    /// Writes `<key>.json` holding `bytes`, atomically and durably: once it returns, the
    /// new file survives a power loss.
    pub fn save_bytes(&self, key: &str, bytes: &[u8]) -> io::Result<()> {
        #[cfg(test)]
        injected(&self.fail_saves)?;
        let tmp = self.dir.join(format!(".{key}.json.tmp"));
        let file = private_file(&tmp)?;
        let mut out = BufWriter::new(&file);
        out.write_all(bytes)?;
        out.flush()?;
        drop(out);
        file.sync_all()?;
        fs::rename(&tmp, self.dir.join(format!("{key}.json")))?;
        sync_dir(&self.dir)
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
        let incomplete = Encoding::ALL.map(|e| incomplete_name(key, e));
        for name in [format!("{key}.json")]
            .iter()
            .chain(&names)
            .chain(&incomplete)
        {
            match fs::rename(self.dir.join(name), expired.join(name)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        sync_dir(&expired)?;
        sync_dir(&self.dir)
    }

    /// Marks game `key`'s stream, here or in `expired/`, as incomplete: renames it to
    /// `<key>.incomplete.events.<jsonl|cbor>`, as it is, so that nothing reads it as the
    /// game's stream or writes to it again. Where it was, if there was one.
    pub fn mark_incomplete(&self, key: &str) -> io::Result<Option<PathBuf>> {
        let Some((path, encoding)) = self.stream_file(key) else {
            return Ok(None);
        };
        let dir = path.parent().unwrap_or(&self.dir).to_path_buf();
        let to = dir.join(incomplete_name(key, encoding));
        fs::rename(&path, &to)?;
        sync_dir(&dir)?;
        Ok(Some(to))
    }

    /// Game `key`'s event stream, here or in `expired/`, with its encoding: the first
    /// found.
    pub fn stream_file(&self, key: &str) -> Option<(PathBuf, Encoding)> {
        stream_file(&self.dir, key)
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
        #[cfg(test)]
        injected(&self.fail_streams)?;
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
            sync_dir(&self.dir)?;
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

/// Game `key`'s event stream in state directory `dir` or its `expired/`, with its
/// encoding: the first found. Never an incomplete one.
pub fn stream_file(dir: &Path, key: &str) -> Option<(PathBuf, Encoding)> {
    [dir.to_path_buf(), dir.join(EXPIRED)]
        .iter()
        .flat_map(|dir| Encoding::ALL.map(|e| (dir.join(format!("{key}{}", e.suffix())), e)))
        .find(|(path, _)| path.exists())
}

/// The name an incomplete stream is kept under.
fn incomplete_name(key: &str, encoding: Encoding) -> String {
    format!("{key}.incomplete{}", encoding.suffix())
}

/// Makes the entries of `dir` (a rename into it) durable.
pub(crate) fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

fn private_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}
