//! Private, exclusively created temporary files beside their final destination.
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub struct TempFile {
    pub file: File,
    path: PathBuf,
}

impl TempFile {
    pub fn new(dest: &Path) -> std::io::Result<Self> {
        use std::os::unix::fs::OpenOptionsExt;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = dest
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)?;
        loop {
            let path = dir.join(format!(
                ".wlens-{}-{}.part",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
            {
                Ok(file) => return Ok(Self { file, path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }

    pub fn persist(&mut self, dest: &Path) -> std::io::Result<()> {
        self.file.sync_all()?;
        std::fs::rename(&self.path, dest)
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn simultaneous_writers_publish_whole_files_and_clean_up() {
        let dir = std::env::temp_dir().join(format!(
            "wlens-atomic-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
        ));
        let dest = dir.join("image");
        let mut a = TempFile::new(&dest).unwrap();
        let mut b = TempFile::new(&dest).unwrap();
        assert_ne!(a.path, b.path);
        a.file.write_all(b"first image").unwrap();
        b.file.write_all(b"second image").unwrap();
        a.persist(&dest).unwrap();
        drop(a);
        assert_eq!(std::fs::read(&dest).unwrap(), b"first image");
        assert!(b.path.exists());
        b.persist(&dest).unwrap();
        drop(b);
        assert_eq!(std::fs::read(&dest).unwrap(), b"second image");
        let abandoned = TempFile::new(&dest).unwrap();
        let path = abandoned.path.clone();
        drop(abandoned);
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
