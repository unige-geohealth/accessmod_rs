//! Server settings, kept in `<data>/settings.json` and written atomically
//! (a temporary file renamed over it).

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{ApiError, ApiResult};

/// File of the settings inside the data directory.
pub const SETTINGS_FILE: &str = "settings.json";

/// Server settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Settings {
    /// Project the client opens when none is named; the demo project when
    /// unset.
    pub default_project: Option<String>,
}

impl Settings {
    /// Settings of `data_dir`: the defaults when the file is missing or
    /// unreadable (with a warning), never an error.
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(SETTINGS_FILE);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                tracing::warn!("{}: {e}; default settings", path.display());
                return Self::default();
            }
        };
        serde_json::from_slice(&bytes).unwrap_or_else(|e| {
            tracing::warn!("{}: {e}; default settings", path.display());
            Self::default()
        })
    }

    /// Write the settings in `data_dir`.
    pub fn save(&self, data_dir: &Path) -> ApiResult<()> {
        let path = data_dir.join(SETTINGS_FILE);
        let temporary = data_dir.join(format!(".{SETTINGS_FILE}.{}", std::process::id()));
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| ApiError::Internal(format!("cannot encode the settings: {e}")))?;
        std::fs::write(&temporary, bytes)
            .and_then(|()| std::fs::rename(&temporary, &path))
            .map_err(|e| {
                let _ = std::fs::remove_file(&temporary);
                ApiError::Internal(format!("cannot write {}: {e}", path.display()))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_and_survive_a_corrupt_file() {
        let dir = std::env::temp_dir().join(format!("am-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        assert_eq!(Settings::load(&dir), Settings::default());
        let settings = Settings {
            default_project: Some("malawi".into()),
        };
        settings.save(&dir).expect("save");
        assert_eq!(Settings::load(&dir), settings);
        std::fs::write(dir.join(SETTINGS_FILE), b"{not json").expect("write");
        assert_eq!(Settings::load(&dir), Settings::default());
        std::fs::remove_dir_all(&dir).ok();
    }
}
