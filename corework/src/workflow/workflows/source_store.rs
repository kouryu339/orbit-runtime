//! Source storage is independent of operating-system paths and execution graphs.
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

pub trait WorkflowSourceStore: Send + Sync {
    fn initialize(&self, root: &Path) -> io::Result<()>;
    fn read(&self, key: &Path) -> io::Result<Vec<u8>>;
    fn write_atomic(&self, key: &Path, bytes: &[u8]) -> io::Result<()>;
    fn remove(&self, key: &Path) -> io::Result<()>;
    fn list(&self, root: &Path) -> io::Result<Vec<PathBuf>>;
    fn exists(&self, key: &Path) -> bool {
        self.read(key).is_ok()
    }
    fn read_text(&self, key: &Path) -> io::Result<String> {
        String::from_utf8(self.read(key)?)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }
}

#[derive(Default)]
pub struct FileWorkflowSourceStore;
impl WorkflowSourceStore for FileWorkflowSourceStore {
    fn exists(&self, key: &Path) -> bool {
        key.exists()
    }
    fn initialize(&self, root: &Path) -> io::Result<()> {
        std::fs::create_dir_all(root)
    }
    fn read(&self, key: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(key)
    }
    fn write_atomic(&self, key: &Path, bytes: &[u8]) -> io::Result<()> {
        super::revision::atomic_write(key, bytes)
    }
    fn remove(&self, key: &Path) -> io::Result<()> {
        std::fs::remove_file(key)
    }
    fn list(&self, root: &Path) -> io::Result<Vec<PathBuf>> {
        std::fs::read_dir(root)?
            .filter_map(|entry| match entry {
                Ok(entry) if entry.path().is_file() => Some(Ok(entry.path())),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }
}

/// Browser-owned content. Durability is supplied by the SDK's transaction adapter.
#[derive(Default)]
pub struct MemoryWorkflowSourceStore {
    documents: parking_lot::RwLock<BTreeMap<PathBuf, Vec<u8>>>,
}
impl MemoryWorkflowSourceStore {
    pub fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        self.documents.read().clone()
    }
    pub fn replace(&self, documents: BTreeMap<PathBuf, Vec<u8>>) {
        *self.documents.write() = documents;
    }
}
impl WorkflowSourceStore for MemoryWorkflowSourceStore {
    fn initialize(&self, _: &Path) -> io::Result<()> {
        Ok(())
    }
    fn read(&self, key: &Path) -> io::Result<Vec<u8>> {
        self.documents
            .read()
            .get(key)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Workflow source not found"))
    }
    fn write_atomic(&self, key: &Path, bytes: &[u8]) -> io::Result<()> {
        self.documents
            .write()
            .insert(key.to_path_buf(), bytes.to_vec());
        Ok(())
    }
    fn remove(&self, key: &Path) -> io::Result<()> {
        self.documents
            .write()
            .remove(key)
            .map(|_| ())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Workflow source not found"))
    }
    fn list(&self, root: &Path) -> io::Result<Vec<PathBuf>> {
        Ok(self
            .documents
            .read()
            .keys()
            .filter(|key| key.parent() == Some(root))
            .cloned()
            .collect())
    }
}
