//! `workspace.json`: versioned layout and session-restore state. An unreadable or too-new file is
//! set aside, never overwritten.

use std::io;
use std::path::Path;
use std::time::SystemTime;

use crate::fsio;

/// Loads the state. `None` when the file is missing, corrupt (set aside as `*.corrupt-<ts>`) or
/// written by a newer version than `supported` (set aside as `*.newer-<ts>`).
pub fn load(path: &Path, supported: u32) -> Option<serde_json::Value> {
    let bytes = std::fs::read(path).ok()?;
    let parsed: Option<serde_json::Value> = serde_json::from_slice(&bytes).ok();
    let version = parsed
        .as_ref()
        .and_then(|v| v.get("version"))
        .and_then(serde_json::Value::as_u64);
    match (parsed, version) {
        (Some(value), Some(v)) if v <= u64::from(supported) => Some(value),
        (Some(_), Some(_)) => {
            let _ = fsio::quarantine(path, "newer", SystemTime::now());
            None
        }
        _ => {
            let _ = fsio::quarantine(path, "corrupt", SystemTime::now());
            None
        }
    }
}

/// Saves `value` (a JSON object) with `"version": version`, atomically.
pub fn save(path: &Path, version: u32, value: &serde_json::Value) -> io::Result<()> {
    let mut value = value.clone();
    let Some(obj) = value.as_object_mut() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "workspace state must be a JSON object",
        ));
    };
    obj.insert("version".into(), serde_json::Value::from(version));
    let bytes = serde_json::to_vec_pretty(&value).map_err(io::Error::other)?;
    fsio::atomic_write(path, &bytes)
}
