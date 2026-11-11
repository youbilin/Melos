//! Best-effort disk cache for artwork, isolated by server and account.
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const MAX_IMAGE: u64 = 4 * 1024 * 1024;
const MAX_CACHE: u64 = 128 * 1024 * 1024;
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

pub struct CoverCache {
    root: PathBuf,
}

// Stable cache filename only; this hash is never used for authentication.
fn key(value: &str) -> String {
    let hash = value.bytes().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}

pub fn is_image(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"\xff\xd8\xff")
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || bytes.starts_with(b"BM")
        || (bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP")
}

impl CoverCache {
    pub fn for_server(server: &str, username: &str) -> Option<Self> {
        dirs::cache_dir().map(|root| Self::at(root.join("melos/covers"), server, username))
    }
    fn at(root: PathBuf, server: &str, username: &str) -> Self {
        Self {
            root: root.join(key(&format!(
                "{}\0{username}",
                server.trim_end_matches('/')
            ))),
        }
    }
    fn path(&self, id: &str, size: u32) -> PathBuf {
        self.root.join(format!("{}-{size}.image", key(id)))
    }
    pub fn read(&self, id: &str, size: u32) -> Option<Vec<u8>> {
        let path = self.path(id, size);
        let metadata = fs::metadata(&path).ok()?;
        if metadata.len() > MAX_IMAGE || metadata.modified().ok()?.elapsed().ok()? > MAX_AGE {
            return None;
        }
        let bytes = fs::read(path).ok()?;
        is_image(&bytes).then_some(bytes)
    }
    pub fn write(&self, id: &str, size: u32, bytes: &[u8]) {
        if bytes.len() as u64 > MAX_IMAGE || !is_image(bytes) {
            return;
        }
        if fs::create_dir_all(&self.root).is_err() {
            return;
        }
        let path = self.path(id, size);
        let temporary = path.with_extension("tmp");
        if fs::write(&temporary, bytes).is_ok() {
            let _ = fs::rename(&temporary, &path);
        }
        self.prune(MAX_CACHE);
    }
    fn prune(&self, limit: u64) {
        let Ok(directory) = fs::read_dir(&self.root) else {
            return;
        };
        let mut files: Vec<_> = directory
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                if path.extension()?.to_str()? != "image" {
                    return None;
                }
                let metadata = entry.metadata().ok()?;
                metadata.is_file().then_some((
                    metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    metadata.len(),
                    path,
                ))
            })
            .collect();
        files.sort_by_key(|(modified, _, _)| *modified);
        let mut bytes: u64 = files.iter().map(|(_, len, _)| len).sum();
        for (_, len, path) in files {
            if bytes <= limit {
                break;
            }
            if fs::remove_file(path).is_ok() {
                bytes -= len;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    fn root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "melos-cache-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    fn cleanup(root: &Path) {
        if let Ok(scopes) = fs::read_dir(root) {
            for scope in scopes.flatten() {
                for file in fs::read_dir(scope.path()).unwrap().flatten() {
                    fs::remove_file(file.path()).unwrap();
                }
                fs::remove_dir(scope.path()).unwrap();
            }
        }
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn artwork_roundtrip_is_isolated_by_server_account_and_size() {
        let root = root();
        let cache = CoverCache::at(root.clone(), "http://one", "alice");
        let bytes = b"\x89PNG\r\n\x1a\nfixture";
        cache.write("../../cover", 240, bytes);
        assert_eq!(
            cache.read("../../cover", 240).as_deref(),
            Some(bytes.as_slice())
        );
        assert!(cache.read("../../cover", 480).is_none());
        assert!(
            CoverCache::at(root.clone(), "http://two", "alice")
                .read("../../cover", 240)
                .is_none()
        );
        assert!(
            CoverCache::at(root.clone(), "http://one", "bob")
                .read("../../cover", 240)
                .is_none()
        );
        cache.write("html-error", 240, b"<html>error</html>");
        assert!(cache.read("html-error", 240).is_none());
        fs::write(cache.path("../../cover", 240), b"corrupted").unwrap();
        assert!(cache.read("../../cover", 240).is_none());
        cleanup(&root);
    }
    #[test]
    fn pruning_enforces_disk_budget_without_removing_other_scopes() {
        let root = root();
        let cache = CoverCache::at(root.clone(), "one", "alice");
        let other = CoverCache::at(root.clone(), "two", "alice");
        let bytes = b"GIF89afixture";
        cache.write("a", 240, bytes);
        cache.write("b", 240, bytes);
        other.write("a", 240, bytes);
        cache.prune(bytes.len() as u64);
        assert_eq!(
            ["a", "b"]
                .iter()
                .filter(|id| cache.read(id, 240).is_some())
                .count(),
            1
        );
        assert!(other.read("a", 240).is_some());
        cleanup(&root);
    }
}
