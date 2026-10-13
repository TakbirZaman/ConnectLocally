use crate::{
    protocol::{Manifest, CHUNK, VERSION},
    store::Store,
};
use anyhow::{ensure, Context, Result};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
pub fn hash(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}
fn identity(f: &File) -> Result<String> {
    let m = f.metadata()?;
    ensure!(m.is_file(), "source is not a regular file");
    Ok(format!(
        "{}:{}",
        m.len(),
        m.modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ))
}
pub fn manifest(path: &Path, id: String) -> Result<Manifest> {
    manifest_cancellable(path, id, None)
}
pub fn manifest_cancellable(
    path: &Path,
    id: String,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Manifest> {
    let mut f = File::open(path).context("source unavailable")?;
    let before = identity(&f)?;
    let size = f.metadata()?.len();
    let mut hashes = Vec::new();
    let mut whole = Sha256::new();
    let mut data = vec![0; CHUNK as usize];
    loop {
        ensure!(
            !cancel.is_some_and(|c| c.is_cancelled()),
            "cancelled while hashing"
        );
        let mut n = 0;
        while n < data.len() {
            let k = f.read(&mut data[n..])?;
            if k == 0 {
                break;
            }
            n += k;
        }
        if n == 0 {
            break;
        }
        hashes.push(hash(&data[..n]));
        whole.update(&data[..n]);
        ensure!(
            hashes.len() <= crate::protocol::MAX_HASHES,
            "file exceeds manifest policy"
        );
    }
    ensure!(identity(&f)? == before, "source changed while hashing");
    let m = Manifest {
        version: VERSION,
        id,
        name: path
            .file_name()
            .context("source name missing")?
            .to_str()
            .context("filename must be UTF-8")?
            .into(),
        size,
        chunk_size: CHUNK,
        hashes,
        whole_hash: hex::encode(whole.finalize()),
        file_identity: before,
    };
    m.validate()?;
    Ok(m)
}
pub fn source_chunk(path: &Path, m: &Manifest, index: u32) -> Result<Vec<u8>> {
    let mut f = File::open(path)?;
    ensure!(
        identity(&f)? == m.file_identity,
        "source changed; start a new transfer"
    );
    let mut bytes = vec![0; m.length(index)?];
    f.seek(SeekFrom::Start(index as u64 * CHUNK as u64))?;
    f.read_exact(&mut bytes)?;
    ensure!(
        hash(&bytes) == m.hashes[index as usize],
        "source content changed"
    );
    Ok(bytes)
}
pub struct Receiver {
    pub m: Manifest,
    pub store: Store,
    pub partial: PathBuf,
    pub destination: PathBuf,
}
impl Receiver {
    pub fn prepare(dir: &Path, m: Manifest, store: Store, peer: &str) -> Result<Self> {
        m.validate()?;
        std::fs::create_dir_all(dir)?;
        let dir = std::fs::canonicalize(dir)?;
        let destination = dir.join(&m.name);
        let partial = dir.join(format!(".{}.localconnect.part", m.id));
        store.register(
            &m,
            peer,
            "receive",
            None,
            destination.to_str().context("non-UTF8 path")?,
            None,
        )?;
        let receiver = Self {
            m,
            store,
            partial,
            destination,
        };
        if receiver.destination.exists() {
            // Only a registered completed/finishing transfer may recover a published file.
            let status = receiver.store.transfer_status(&receiver.m.id)?;
            ensure!(
                status == "finishing" || status == "complete",
                "destination exists; choose another folder or rename source"
            );
            ensure!(
                receiver.verify_path(&receiver.destination)?,
                "destination collision"
            );
            return Ok(receiver);
        }
        if receiver.partial.exists() {
            ensure!(
                std::fs::symlink_metadata(&receiver.partial)?.is_file(),
                "partial must be a regular file, not a symlink or device"
            );
        } else {
            let f = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&receiver.partial)?;
            f.set_len(receiver.m.size)?;
            f.sync_all()?;
            sync_dir(&dir)?;
        }
        receiver.store.status(&receiver.m.id, "receiving", None)?;
        Ok(receiver)
    }
    fn file(&self) -> Result<File> {
        ensure!(
            std::fs::symlink_metadata(&self.partial)?.is_file(),
            "partial must be a regular file, not a symlink or device"
        );
        Ok(OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.partial)?)
    }
    pub fn reconcile(&self) -> Result<Vec<u32>> {
        if self.destination.exists() && self.verify_path(&self.destination)? {
            return Ok(vec![]);
        }
        let mut f = self.file()?;
        let mut verified = std::collections::HashSet::new();
        for idx in self.store.indexes(&self.m.id)? {
            let valid = (|| -> Result<bool> {
                let mut b = vec![0; self.m.length(idx)?];
                f.seek(SeekFrom::Start(idx as u64 * CHUNK as u64))?;
                f.read_exact(&mut b)?;
                Ok(hash(&b) == self.m.hashes[idx as usize])
            })();
            if matches!(valid, Ok(true)) {
                verified.insert(idx);
            } else {
                self.store.invalidate(&self.m.id, idx)?;
            }
        }
        Ok((0..self.m.hashes.len() as u32)
            .filter(|i| !verified.contains(i))
            .collect())
    }
    pub fn accept(&self, index: u32, data: &[u8]) -> Result<u64> {
        ensure!(data.len() == self.m.length(index)?, "chunk length mismatch");
        ensure!(
            hash(data) == self.m.hashes[index as usize],
            "corrupted chunk"
        );
        // Rewriting duplicates is safe and also repairs stale disk state.
        let mut f = self.file()?;
        f.seek(SeekFrom::Start(index as u64 * CHUNK as u64))?;
        f.write_all(data)?;
        f.sync_data()?;
        // FULL synchronous SQLite commit occurs AFTER file durability, BEFORE ACK.
        self.store
            .chunk(&self.m.id, index, &self.m.hashes[index as usize])?;
        self.verified_bytes()
    }
    pub fn verified_bytes(&self) -> Result<u64> {
        self.store
            .indexes(&self.m.id)?
            .iter()
            .try_fold(0, |n, i| Ok(n + self.m.length(*i)? as u64))
    }
    fn verify_path(&self, path: &Path) -> Result<bool> {
        if !std::fs::symlink_metadata(path)?.is_file() {
            return Ok(false);
        }
        let mut f = File::open(path)?;
        if f.metadata()?.len() != self.m.size {
            return Ok(false);
        }
        let mut h = Sha256::new();
        let mut b = vec![0; 1024 * 1024];
        loop {
            let n = f.read(&mut b)?;
            if n == 0 {
                break;
            }
            h.update(&b[..n]);
        }
        Ok(hex::encode(h.finalize()) == self.m.whole_hash)
    }
    pub fn finish(&self) -> Result<()> {
        if self.destination.exists() {
            ensure!(
                self.verify_path(&self.destination)?,
                "existing destination mismatch"
            );
            self.store.status(&self.m.id, "complete", None)?;
            self.cleanup_partial()?;
            return Ok(());
        }
        ensure!(self.reconcile()?.is_empty(), "missing verified chunks");
        ensure!(self.verify_path(&self.partial)?, "whole-file hash mismatch");
        self.file()?.sync_all()?;
        self.store.status(&self.m.id, "finishing", None)?;
        // Atomic no-replace publication on same filesystem. No overwrite race.
        // hard_link is deliberately used instead of rename(overwrites on Unix).
        std::fs::hard_link(&self.partial, &self.destination)
            .context("atomic no-overwrite publication unavailable")?;
        sync_dir(self.destination.parent().unwrap())?;
        self.store.status(&self.m.id, "complete", None)?;
        self.cleanup_partial()?;
        Ok(())
    }
    fn cleanup_partial(&self) -> Result<()> {
        match std::fs::symlink_metadata(&self.partial) {
            Ok(metadata) => {
                ensure!(
                    metadata.is_file(),
                    "partial cleanup requires a regular file"
                );
                std::fs::remove_file(&self.partial)?;
                sync_dir(self.destination.parent().unwrap())?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        Ok(())
    }
}
fn sync_dir(dir: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        File::open(dir)?.sync_all()?;
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_rejects_changed_destination() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("source.bin");
        std::fs::write(&src, b"bound destination").unwrap();
        let m = manifest(&src, uuid::Uuid::new_v4().to_string()).unwrap();
        let store = Store::open(&d.path().join("db")).unwrap();
        let original =
            Receiver::prepare(&d.path().join("original"), m.clone(), store.clone(), "peer")
                .unwrap();
        original.accept(0, b"bound destination").unwrap();
        let other = d.path().join("other");
        assert!(Receiver::prepare(&other, m.clone(), store, "peer").is_err());
        assert!(!other.join(format!(".{}.localconnect.part", m.id)).exists());
        assert!(original.reconcile().unwrap().is_empty());
        original.finish().unwrap();
    }
    #[test]
    fn failed_recovery_commit_is_not_accepted() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("source.bin");
        std::fs::write(&src, b"durable bytes").unwrap();
        let m = manifest(&src, uuid::Uuid::new_v4().to_string()).unwrap();
        let store = Store::open(&d.path().join("db")).unwrap();
        let receiver =
            Receiver::prepare(&d.path().join("out"), m.clone(), store.clone(), "peer").unwrap();
        let db = rusqlite::Connection::open(&store.0).unwrap();
        // Deterministic SQLite commit-stage fault; not a physical ENOSPC test.
        db.execute_batch("CREATE TRIGGER reject_chunk BEFORE INSERT ON chunks BEGIN SELECT RAISE(ABORT, 'injected recovery commit failure'); END;").unwrap();
        assert!(receiver.accept(0, b"durable bytes").is_err());
        assert!(store.indexes(&m.id).unwrap().is_empty());
        assert_eq!(receiver.reconcile().unwrap(), vec![0]);
        assert!(receiver.finish().is_err());
        db.execute_batch("DROP TRIGGER reject_chunk;").unwrap();
        drop(receiver);
        let receiver = Receiver::prepare(&d.path().join("out"), m, store, "peer").unwrap();
        assert_eq!(receiver.reconcile().unwrap(), vec![0]);
        receiver.accept(0, b"durable bytes").unwrap();
        receiver.finish().unwrap();
        assert_eq!(
            std::fs::read(receiver.destination).unwrap(),
            b"durable bytes"
        );
    }
    #[test]
    fn non_regular_partial_is_rejected() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("source.bin");
        std::fs::write(&src, b"safe").unwrap();
        let m = manifest(&src, uuid::Uuid::new_v4().to_string()).unwrap();
        let store = Store::open(&d.path().join("db")).unwrap();
        let receiver =
            Receiver::prepare(&d.path().join("out"), m.clone(), store.clone(), "peer").unwrap();
        std::fs::remove_file(&receiver.partial).unwrap();
        std::fs::create_dir(&receiver.partial).unwrap();
        assert!(receiver.accept(0, b"safe").is_err());
        assert!(Receiver::prepare(&d.path().join("out"), m, store.clone(), "peer").is_err());
        assert!(store.indexes(&receiver.m.id).unwrap().is_empty());
        #[cfg(unix)]
        {
            std::fs::remove_dir(&receiver.partial).unwrap();
            std::os::unix::fs::symlink(&src, &receiver.partial).unwrap();
            assert!(receiver.accept(0, b"safe").is_err());
            assert_eq!(std::fs::read(&src).unwrap(), b"safe");
        }
    }
    #[test]
    fn corrupt_duplicate_restart() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("source.bin");
        std::fs::write(&src, b"hello recovery").unwrap();
        let m = manifest(&src, uuid::Uuid::new_v4().to_string()).unwrap();
        let out = d.path().join("out");
        let s = Store::open(&d.path().join("db")).unwrap();
        let r = Receiver::prepare(&out, m.clone(), s.clone(), "peer").unwrap();
        assert!(r.accept(0, b"bad").is_err());
        let bytes = source_chunk(&src, &m, 0).unwrap();
        assert_eq!(r.accept(0, &bytes).unwrap(), 14);
        assert_eq!(r.accept(0, &bytes).unwrap(), 14);
        drop(r);
        let r = Receiver::prepare(&out, m, s, "peer").unwrap();
        assert!(r.reconcile().unwrap().is_empty());
        std::fs::write(&r.partial, b"corrupt").unwrap();
        assert_eq!(r.reconcile().unwrap(), vec![0]);
        r.accept(0, &bytes).unwrap();
        r.finish().unwrap();
        assert_eq!(std::fs::read(r.destination).unwrap(), bytes);
    }
    #[test]
    fn empty_and_collision() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("empty");
        std::fs::write(&p, []).unwrap();
        let m = manifest(&p, uuid::Uuid::new_v4().to_string()).unwrap();
        let s = Store::open(&d.path().join("db")).unwrap();
        let r = Receiver::prepare(&d.path().join("out"), m, s.clone(), "peer").unwrap();
        r.finish().unwrap();
        assert_eq!(std::fs::metadata(&r.destination).unwrap().len(), 0);
        let m = manifest(&p, uuid::Uuid::new_v4().to_string()).unwrap();
        assert!(Receiver::prepare(&d.path().join("out"), m, s, "peer").is_err());
    }
    #[test]
    fn published_file_recovers_failed_completion_commit_and_cleans_partial() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("source.bin");
        let bytes = b"published durable data";
        std::fs::write(&src, bytes).unwrap();
        let m = manifest(&src, uuid::Uuid::new_v4().to_string()).unwrap();
        let db_path = d.path().join("db");
        let store = Store::open(&db_path).unwrap();
        let out = d.path().join("out");
        let receiver = Receiver::prepare(&out, m.clone(), store.clone(), "peer").unwrap();
        receiver.accept(0, bytes).unwrap();
        let db = rusqlite::Connection::open(&store.0).unwrap();
        db.execute_batch(
            "CREATE TRIGGER reject_completion BEFORE UPDATE OF status ON transfers
             WHEN NEW.status='complete'
             BEGIN SELECT RAISE(ABORT, 'injected completion commit failure'); END;",
        )
        .unwrap();
        assert!(receiver.finish().is_err());
        assert_eq!(store.transfer_status(&m.id).unwrap(), "finishing");
        assert_eq!(std::fs::read(&receiver.destination).unwrap(), bytes);
        assert!(receiver.partial.exists());
        db.execute_batch("DROP TRIGGER reject_completion;").unwrap();
        drop(db);
        drop(receiver);
        drop(store);
        let store = Store::open(&db_path).unwrap();
        let recovered = Receiver::prepare(&out, m.clone(), store.clone(), "peer").unwrap();
        assert!(recovered.reconcile().unwrap().is_empty());
        recovered.finish().unwrap();
        assert_eq!(store.transfer_status(&m.id).unwrap(), "complete");
        assert!(!recovered.partial.exists());
        assert_eq!(std::fs::read(&recovered.destination).unwrap(), bytes);
        recovered.finish().unwrap();
    }
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires isolated size-limited tmpfs; run through Linux CI"]
    fn disk_full_does_not_verify_chunk_and_recovers() {
        let volume = std::env::var("LOCALCONNECT_FAILURE_VOLUME").unwrap();
        let volume = PathBuf::from(volume).canonicalize().unwrap();
        let marker = volume.join(".localconnect-disposable-volume");
        assert!(marker.is_file(), "refusing an unmarked volume");
        let work = tempfile::tempdir_in(&volume).unwrap();
        let state = tempfile::tempdir().unwrap();
        let bytes = vec![42u8; CHUNK as usize];
        let source = state.path().join("source.bin");
        std::fs::write(&source, &bytes).unwrap();
        let m = manifest(&source, uuid::Uuid::new_v4().to_string()).unwrap();
        let db_path = state.path().join("db");
        let store = Store::open(&db_path).unwrap();
        let out = work.path().join("out");
        let receiver = Receiver::prepare(&out, m.clone(), store.clone(), "peer").unwrap();
        let filler_path = work.path().join("filler");
        let mut filler = File::create(&filler_path).unwrap();
        let block = vec![0u8; 1024 * 1024];
        let mut filled = 0;
        loop {
            match filler.write_all(&block) {
                Ok(()) => {
                    filled += block.len();
                    assert!(filled <= 32 * 1024 * 1024, "volume exceeds test limit");
                }
                Err(error) => {
                    assert_eq!(error.raw_os_error(), Some(28));
                    break;
                }
            }
        }
        let error = receiver.accept(0, &bytes).unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .raw_os_error(),
            Some(28)
        );
        assert!(store.indexes(&m.id).unwrap().is_empty());
        assert!(receiver.finish().is_err());
        assert!(!receiver.destination.exists());
        drop(filler);
        std::fs::remove_file(filler_path).unwrap();
        drop(receiver);
        drop(store);
        let store = Store::open(&db_path).unwrap();
        let recovered = Receiver::prepare(&out, m.clone(), store.clone(), "peer").unwrap();
        assert_eq!(recovered.reconcile().unwrap(), vec![0]);
        recovered.accept(0, &bytes).unwrap();
        recovered.finish().unwrap();
        assert_eq!(std::fs::read(&recovered.destination).unwrap(), bytes);
        assert_eq!(store.transfer_status(&m.id).unwrap(), "complete");
    }
    #[test]
    fn source_mutation() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x");
        std::fs::write(&p, b"old").unwrap();
        let m = manifest(&p, uuid::Uuid::new_v4().to_string()).unwrap();
        std::fs::write(&p, b"new").unwrap();
        assert!(source_chunk(&p, &m, 0).is_err());
    }
    #[test]
    fn greater_than_4gib_metadata() {
        let count = 1025;
        let m = Manifest {
            version: VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            name: "large.bin".into(),
            size: CHUNK as u64 * count,
            chunk_size: CHUNK,
            hashes: vec!["00".repeat(32); count as usize],
            whole_hash: "00".repeat(32),
            file_identity: "test".into(),
        };
        m.validate().unwrap();
        assert_eq!(m.length(1024).unwrap(), CHUNK as usize);
    }
}
