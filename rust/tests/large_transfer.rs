//! Opt-in Linux resource acceptance check; writes over 4 GiB to a temporary disk.
#![cfg(target_os = "linux")]
use localconnect_core::engine::{Config, Engine, Peer};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
fn config(path: &Path, name: &str) -> Config {
    Config {
        data_dir: path.join("state").display().to_string(),
        receive_dir: path.join("received").display().to_string(),
        name: name.into(),
        port: 0,
        in_flight: 4,
    }
}
fn rss() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse::<u64>().ok())
        })
        .unwrap()
        * 1024
}
fn digest(path: &Path) -> String {
    let mut file = std::fs::File::open(path).unwrap();
    let mut bytes = vec![0; 1024 * 1024];
    let mut hash = Sha256::new();
    loop {
        let count = file.read(&mut bytes).unwrap();
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    hex::encode(hash.finalize())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires >5 GiB free disk and several minutes; run explicitly"]
async fn larger_than_4gib_verified_with_bounded_rss() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let sender = Engine::start(config(a.path(), "large sender"))
        .await
        .unwrap();
    let receiver = Engine::start(config(b.path(), "large receiver"))
        .await
        .unwrap();
    // Trust fixture: explicit pairing itself is covered in the network suite.
    sender
        .store
        .trust(&receiver.identity.fingerprint, "test fixture")
        .unwrap();
    receiver
        .store
        .trust(&sender.identity.fingerprint, "test fixture")
        .unwrap();
    let size = (1u64 << 32) + 17;
    let source = a.path().join("large.bin");
    let mut file = std::fs::File::create(&source).unwrap();
    file.set_len(size).unwrap();
    file.write_all(b"begin").unwrap();
    file.seek(SeekFrom::End(-3)).unwrap();
    file.write_all(b"end").unwrap();
    file.sync_all().unwrap();
    drop(file);
    let baseline = rss();
    let peak = Arc::new(AtomicU64::new(baseline));
    let stop = Arc::new(AtomicBool::new(false));
    let sampler_peak = peak.clone();
    let sampler_stop = stop.clone();
    let sampler = std::thread::spawn(move || {
        while !sampler_stop.load(Ordering::Relaxed) {
            sampler_peak.fetch_max(rss(), Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(50));
        }
    });
    let peer = Peer {
        fingerprint: receiver.identity.fingerprint.clone(),
        name: "receiver".into(),
        address: format!("127.0.0.1:{}", receiver.port),
    };
    let result = tokio::time::timeout(Duration::from_secs(900), async {
        let id = sender
            .send(source.display().to_string(), peer)
            .await
            .unwrap();
        loop {
            let status = sender.store.transfer_status(&id).unwrap();
            assert!(
                !matches!(status.as_str(), "failed" | "cancelled"),
                "transfer failed: {}",
                sender.store.history().unwrap()
            );
            if status == "complete" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let destination = b.path().join("received/large.bin");
        assert_eq!(std::fs::metadata(&destination).unwrap().len(), size);
        let (manifest, _, _, _) = sender.store.outbound(&id).unwrap();
        let hash = tokio::task::spawn_blocking(move || digest(&destination))
            .await
            .unwrap();
        assert_eq!(hash, manifest.whole_hash);
        println!("verified {size} bytes; SHA-256 {hash}");
    })
    .await;
    stop.store(true, Ordering::Relaxed);
    sampler.join().unwrap();
    sender.shutdown().await;
    receiver.shutdown().await;
    result.unwrap();
    let maximum = peak.load(Ordering::Relaxed);
    println!(
        "RSS baseline={} peak={} delta={} bytes (sender+receiver in one process)",
        baseline,
        maximum,
        maximum.saturating_sub(baseline)
    );
    assert!(
        maximum.saturating_sub(baseline) < 256 * 1024 * 1024,
        "RSS increase exceeded 256 MiB acceptance bound"
    );
}
