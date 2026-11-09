//! Native per-user configuration paths; only presentation settings and message drafts live here.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub dark: bool,
    pub group_folders: bool,
    pub soft_wrap: bool,
    pub drafts: BTreeMap<PathBuf, String>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            dark: false,
            group_folders: true,
            soft_wrap: true,
            drafts: BTreeMap::new(),
        }
    }
}
impl Settings {
    pub fn defaults() -> Self {
        Self::default()
    }
    pub fn path() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os("DIFLY_CONFIG_DIR") {
            return Ok(PathBuf::from(path).join("settings.json"));
        }
        directories::ProjectDirs::from("dev", "tommica", "difly")
            .map(|p| p.config_dir().join("settings.json"))
            .context("Could not determine the user configuration directory")
    }
    pub fn load(path: &Path) -> Result<Self> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).context("Read Difly settings"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::defaults()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn load_migrating(path: &Path) -> Result<Self> {
        if path.exists() || std::env::var_os("DIFLY_CONFIG_DIR").is_some() {
            return Self::load(path);
        }
        let mut settings = Self::defaults();
        if let Some(old) = directories::ProjectDirs::from("", "", "difly") {
            let file = old.data_dir().join("app.ron");
            if let Ok(data) = fs::read_to_string(file)
                && let Ok(values) = ron::from_str::<BTreeMap<String, String>>(&data)
            {
                settings.dark = values.get("theme").is_some_and(|s| s == "dark");
            }
        }
        Ok(settings)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path.parent().context("Settings path needs a parent")?;
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        use std::io::Write;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
        file.flush()?;
        file.persist(path).map_err(|e| e.error)?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_settings_default_to_light_and_preferences_and_drafts_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut settings = Settings::load(&path).unwrap();
        assert!(!settings.dark && settings.group_folders && settings.soft_wrap);
        settings.dark = true;
        settings
            .drafts
            .insert(PathBuf::from("repo"), "message".into());
        settings.save(&path).unwrap();
        let saved = Settings::load(&path).unwrap();
        assert!(saved.dark);
        assert_eq!(saved.drafts[Path::new("repo")], "message");
        fs::write(&path, "bad settings").unwrap();
        assert!(Settings::load(&path).is_err());
    }
    #[test]
    fn older_settings_with_missing_fields_keep_new_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"dark":true}"#).unwrap();
        assert!(settings.dark && settings.soft_wrap && settings.group_folders);
    }
}
