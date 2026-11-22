//! Remembered state between runs (`~/.local/state/wlens/state.json`):
//! per script, the options changed while running (menus, keys, `set`) and
//! the script's own `wlens.store` values.
//!
//! ```json
//! { "scripts": { "/path/to/gallery.lua": { "options": { "gallery.fit": "contain" },
//!                                         "store": { "sort": "date" } } } }
//! ```

use std::path::PathBuf;

use serde_json::{Map, Value};

fn path() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::cache_dir)
        .map(|d| d.join("wlens").join("state.json"))
}

fn load_all() -> Value {
    path()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Map::new()))
}

fn save_all(p: &std::path::Path, v: &Value) -> anyhow::Result<()> {
    use std::io::Write;
    let mut tmp = super::atomic::TempFile::new(p)?;
    tmp.file
        .write_all(serde_json::to_string_pretty(v)?.as_bytes())?;
    tmp.persist(p)?;
    Ok(())
}

/// One script's section ("options" or "store").
pub fn section(script: &str, which: &str) -> Map<String, Value> {
    load_all()
        .get("scripts")
        .and_then(|s| s.get(script))
        .and_then(|s| s.get(which))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// Set (or with `None`, remove) one key in a script's section and save.
pub fn put(script: &str, which: &str, key: &str, value: Option<Value>) {
    if let Err(e) = put_locked(script, which, key, value) {
        log::warn!("couldn't save remembered state: {e:#}");
    }
}

fn put_locked(script: &str, which: &str, key: &str, value: Option<Value>) -> anyhow::Result<()> {
    let Some(p) = path() else { return Ok(()) };
    put_at(&p, script, which, key, value)
}

fn put_at(
    p: &std::path::Path,
    script: &str,
    which: &str,
    key: &str,
    value: Option<Value>,
) -> anyhow::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::create_dir_all(p.parent().unwrap())?;
    // Lock a stable sidecar inode, never the state file replaced by rename.
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(p.with_extension("lock"))?;
    lock.lock()?;
    // Keep the lock through reading, merging, and publishing the new file.
    let mut all = match std::fs::read_to_string(p) {
        Ok(s) => serde_json::from_str::<Value>(&s)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Map::new()),
        Err(e) => return Err(e.into()),
    };
    anyhow::ensure!(all.is_object(), "remembered state must be an object");
    let root = all.as_object_mut().unwrap();
    let scripts = root
        .entry("scripts")
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(scripts) = scripts.as_object_mut() else {
        anyhow::bail!("invalid remembered state section");
    };
    let mine = scripts
        .entry(script)
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(mine) = mine.as_object_mut() else {
        anyhow::bail!("invalid remembered state section");
    };
    let sec = mine
        .entry(which)
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(sec) = sec.as_object_mut() else {
        anyhow::bail!("invalid remembered state section");
    };
    match value {
        Some(v) => {
            sec.insert(key.to_string(), v);
        }
        None => {
            sec.remove(key);
        }
    }
    save_all(p, &all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_writers_merge_and_invalid_state_is_preserved() {
        let dir = std::env::temp_dir().join(format!(
            "wlens-state-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        std::thread::scope(|scope| {
            for i in 0..12 {
                let path = &path;
                scope.spawn(move || {
                    for n in 0..5 {
                        put_at(
                            path,
                            "script",
                            "store",
                            &format!("key-{i}"),
                            Some(Value::from(n)),
                        )
                        .unwrap();
                    }
                });
            }
        });
        let all: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let values = all["scripts"]["script"]["store"].as_object().unwrap();
        assert_eq!(values.len(), 12);
        assert!(values.values().all(|v| v == &Value::from(4)));
        std::fs::write(&path, "broken JSON").unwrap();
        assert!(put_at(&path, "script", "store", "key", Some(Value::from(1))).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken JSON");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
