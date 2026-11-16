//! Operator/account choices are flat TOML, independently located from game state.
use dominion_online::transport::{Preference, PreferenceValue};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Choices {
    pub preferred_background: Option<i32>,
    pub move_kingdom: Option<bool>,
    pub confirm_actions: Option<bool>,
    pub client_language: Option<i32>,
    pub animation_type: Option<i32>,
    pub chat_visible: Option<bool>,
    pub notification_sounds: Option<bool>,
    pub victory_or_defeat_sound: Option<bool>,
    pub use_autobuy: Option<bool>,
}
impl Choices {
    fn set(&mut self, pref: &Preference) -> Result<(), String> {
        match (&pref.value, pref.id) {
            (PreferenceValue::Int(v), 2) if (0..=29).contains(v) => {
                self.preferred_background = Some(*v)
            }
            (PreferenceValue::Boolean(v), 5) => self.move_kingdom = Some(*v),
            (PreferenceValue::Boolean(v), 6) => self.confirm_actions = Some(*v),
            (PreferenceValue::Boolean(v), 11) => self.use_autobuy = Some(*v),
            (PreferenceValue::Int(v), 14) if (0..=1).contains(v) => self.client_language = Some(*v),
            (PreferenceValue::Boolean(v), 15) => self.notification_sounds = Some(*v),
            (PreferenceValue::Boolean(v), 16) => self.victory_or_defeat_sound = Some(*v),
            (PreferenceValue::Boolean(v), 25) => self.chat_visible = Some(*v),
            (PreferenceValue::Int(v), 30) if (0..=3).contains(v) => self.animation_type = Some(*v),
            _ => return Err("preference has no supported flat-file mapping".into()),
        }
        Ok(())
    }
    pub(crate) fn preferences(&self) -> Vec<Preference> {
        let mut out = vec![];
        for (id, value) in [
            (2, self.preferred_background),
            (14, self.client_language),
            (30, self.animation_type),
        ] {
            if let Some(value) = value {
                out.push(Preference {
                    id,
                    value: PreferenceValue::Int(value),
                });
            }
        }
        for (id, value) in [
            (5, self.move_kingdom),
            (6, self.confirm_actions),
            (11, self.use_autobuy),
            (15, self.notification_sounds),
            (16, self.victory_or_defeat_sound),
            (25, self.chat_visible),
        ] {
            if let Some(value) = value {
                out.push(Preference {
                    id,
                    value: PreferenceValue::Boolean(value),
                });
            }
        }
        out
    }
}
#[derive(Debug)]
pub(crate) struct Store {
    dir: Option<PathBuf>,
    lock: Mutex<()>,
}
impl Store {
    pub(crate) fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            lock: Mutex::new(()),
        }
    }
    fn path(&self, account: &str) -> Result<PathBuf, String> {
        super::identity::number(account, 'u')?;
        self.dir
            .as_ref()
            .map(|d| d.join(format!("{account}.toml")))
            .ok_or_else(|| "official preferences directory is not configured".into())
    }
    pub(crate) fn read(&self, account: &str) -> Result<Choices, String> {
        let Some(_) = self.dir else {
            return Ok(Choices::default());
        };
        let path = self.path(account)?;
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let choices: Choices =
                    toml::from_str(&text).map_err(|_| "invalid account preferences file")?;
                let mut checked = Choices::default();
                for preference in choices.preferences() {
                    checked.set(&preference)?;
                }
                Ok(checked)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Choices::default()),
            Err(_) => Err("account preferences unavailable".into()),
        }
    }
    pub(crate) fn update(&self, account: &str, pref: &Preference) -> Result<(), String> {
        let _lock = crate::hub::lock(&self.lock);
        let path = self.path(account)?;
        let mut choices = self.read(account)?;
        choices.set(pref)?;
        write(
            &path,
            &toml::to_string(&choices).map_err(|_| "cannot encode preferences")?,
        )
        .map_err(|_| "cannot persist account preferences".into())
    }
}
fn write(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().unwrap())?;
    let temp = path.with_extension(format!("toml.{:016x}.new", rand::random::<u64>()));
    let result = (|| {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_survive_restart_as_flat_choices() {
        let dir = tempfile::tempdir().unwrap();
        let pref = Preference {
            id: 14,
            value: PreferenceValue::Int(1),
        };
        Store::new(Some(dir.path().into()))
            .update("u1", &pref)
            .unwrap();
        assert_eq!(
            Store::new(Some(dir.path().into()))
                .read("u1")
                .unwrap()
                .preferences(),
            vec![pref]
        );
        assert!(
            std::fs::read_to_string(dir.path().join("u1.toml"))
                .unwrap()
                .contains("client_language = 1")
        );
        assert!(
            Store::new(None)
                .update(
                    "u1",
                    &Preference {
                        id: 0,
                        value: PreferenceValue::Deleted
                    }
                )
                .is_err()
        );
        std::fs::write(dir.path().join("u1.toml"), "client_language = 900\n").unwrap();
        assert!(Store::new(Some(dir.path().into())).read("u1").is_err());
    }
}
