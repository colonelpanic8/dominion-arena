//! The state directory: JSON files rewritten whole, each write atomic. Games are
//! `<game id>.json` at the top; accounts, sessions and finished games' records each have
//! a subdirectory.

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

pub struct Store {
    dir: PathBuf,
    /// Held locked while the directory is in use, so two servers never share it.
    _lock: fs::File,
}

fn create_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

impl Store {
    pub fn open(dir: &Path) -> io::Result<Store> {
        create_dir(dir)?;
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

    fn path(&self, sub: Option<&str>) -> PathBuf {
        match sub {
            Some(sub) => self.dir.join(sub),
            None => self.dir.clone(),
        }
    }

    /// Writes `<sub>/<key>.json`.
    pub fn save(&self, sub: Option<&str>, key: &str, value: &impl Serialize) -> io::Result<()> {
        let dir = self.path(sub);
        if sub.is_some() {
            create_dir(&dir)?;
        }
        let tmp = dir.join(format!(".{key}.json.tmp"));
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
        fs::rename(&tmp, dir.join(format!("{key}.json")))
    }

    pub fn remove(&self, sub: Option<&str>, key: &str) -> io::Result<()> {
        match fs::remove_file(self.path(sub).join(format!("{key}.json"))) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// Every `<sub>/<key>.json`, by key; what couldn't be read comes with why.
    pub fn load<T: DeserializeOwned>(
        &self,
        sub: &str,
    ) -> io::Result<Vec<(String, Result<T, String>)>> {
        let dir = self.dir.join(sub);
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        let mut out = vec![];
        for entry in entries {
            let path = entry?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') {
                if name.ends_with(".json.tmp") {
                    fs::remove_file(&path)?;
                }
                continue;
            }
            let Some(key) = name.strip_suffix(".json") else {
                continue;
            };
            let value = fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|e| e.to_string()));
            out.push((key.to_string(), value));
        }
        Ok(out)
    }
}
