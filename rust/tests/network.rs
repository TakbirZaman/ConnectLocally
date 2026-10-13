use localconnect_core::{
    engine::{Config, Engine},
    identity::Identity,
    protocol::*,
    transfer,
};
use std::time::Duration;
fn cfg(dir: &std::path::Path, name: &str) -> Config {
    Config {
        data_dir: dir.join("state").to_str().unwrap().into(),
        receive_dir: dir.join("received").to_str().unwrap().into(),
        name: name.into(),
        port: 0,
        in_flight: 4,
    }
}
#[tokio::test]
async fn invalid_window_is_rejected_before_startup() {
    let d = tempfile::tempdir().unwrap();
    for window in [0, 9] {
        let mut config = cfg(d.path(), "test");
        config.in_flight = window;
        assert!(Engine::start(config).await.is_err());
    }
    assert!(!d.path().join("state").exists());
}
#[tokio::test(flavor = "multi_thread")]
async fn receiver_limit_rejects_trusted_bulk_without_partial_file() {
    use localconnect_core::engine::TransferLimits;
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let aa = Engine::start(cfg(a.path(), "A")).await.unwrap();
    let bb = Engine::start(cfg(b.path(), "B")).await.unwrap();
    bb.configure_transfer(TransferLimits {
        in_flight: 4,
        max_file_bytes: 3,
        max_reserved_bytes: 6,
    })
    .await
    .unwrap();
    bb.store
        .trust(&aa.identity.fingerprint, "A test fixture")
        .unwrap();
    let (_, client) = aa.identity.configs().unwrap();
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", bb.port))
        .await
        .unwrap();
    let mut socket = tokio_rustls::TlsConnector::from(client)
        .connect(
            rustls::pki_types::ServerName::try_from("localconnect.local")
                .unwrap()
                .to_owned(),
            tcp,
        )
        .await
        .unwrap();
    read_message(&mut socket).await.unwrap();
    let source = a.path().join("limit.bin");
    std::fs::write(&source, b"four").unwrap();
    let manifest = transfer::manifest(&source, uuid::Uuid::new_v4().to_string()).unwrap();
    let id = manifest.id.clone();
    write_message(&mut socket, &Message::Bulk { manifest })
        .await
        .unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), read_message(&mut socket))
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(response,Message::Error{code,..} if code=="quota_exceeded"));
    assert!(!b.path().join("received/limit.bin").exists());
    assert!(!b
        .path()
        .join(format!("received/.{id}.localconnect.part"))
        .exists());
    aa.shutdown().await;
    bb.shutdown().await;
}
#[tokio::test(flavor = "multi_thread")]
async fn cleanup_releases_reservation_and_retains_original_source() {
    let d = tempfile::tempdir().unwrap();
    let engine = Engine::start(cfg(d.path(), "cleanup")).await.unwrap();
    let source = d.path().join("keep.bin");
    std::fs::write(&source, b"retained").unwrap();
    let manifest = transfer::manifest(&source, uuid::Uuid::new_v4().to_string()).unwrap();
    let receiver = transfer::Receiver::prepare(
        &d.path().join("received"),
        manifest.clone(),
        engine.store.clone(),
        "fixture",
    )
    .unwrap();
    assert_eq!(engine.store.receive_reservations("").unwrap(), (8, 1));
    engine.cleanup_transfer(manifest.id.clone()).await.unwrap();
    assert!(!receiver.partial.exists());
    assert_eq!(std::fs::read(source).unwrap(), b"retained");
    assert_eq!(engine.store.receive_reservations("").unwrap(), (0, 0));
    assert!(engine.store.transfer_status(&manifest.id).is_err());
    engine.shutdown().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mutual_pair_transfer_and_restart() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let ca = cfg(a.path(), "A");
    let cb = cfg(b.path(), "B");
    let aa = Engine::start(ca.clone()).await.unwrap();
    let bb = Engine::start(cb.clone()).await.unwrap();
    let mut events = bb.events.subscribe();
    let approve = bb.clone();
    let waiter = tokio::spawn(async move {
        loop {
            let event = events.recv().await.unwrap();
            if event.kind == "pair_request" {
                let p: serde_json::Value = serde_json::from_str(&event.payload).unwrap();
                approve
                    .approve(p["fingerprint"].as_str().unwrap().into(), true)
                    .await
                    .unwrap();
                break;
            }
        }
    });
    let addr = format!("127.0.0.1:{}", bb.port);
    let peer = aa.probe(addr.clone()).await.unwrap();
    assert_eq!(peer.fingerprint, bb.identity.fingerprint);
    let peer = aa.pair(addr, peer.fingerprint).await.unwrap();
    waiter.await.unwrap();
    let src = a.path().join("example.bin");
    let data = vec![7u8; CHUNK as usize * 5 + 19];
    std::fs::write(&src, &data).unwrap();
    let id = aa.send(src.to_str().unwrap().into(), peer).await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if aa.store.transfer_status(&id).unwrap() == "complete" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await
        }
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(b.path().join("received/example.bin")).unwrap(),
        data
    );
    assert_eq!(
        transfer::hash(&data),
        transfer::hash(&std::fs::read(b.path().join("received/example.bin")).unwrap())
    );
    aa.shutdown().await;
    bb.shutdown().await;
    let restarted = Engine::start(cb).await.unwrap();
    assert_eq!(
        restarted.identity.fingerprint,
        Identity::load(&b.path().join("state")).unwrap().fingerprint
    );
    assert!(restarted.store.trusted(&aa.identity.fingerprint).unwrap());
    restarted.shutdown().await;
}
#[tokio::test(flavor = "multi_thread")]
async fn unpaired_bulk_is_rejected() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let aa = Engine::start(cfg(a.path(), "A")).await.unwrap();
    let bb = Engine::start(cfg(b.path(), "B")).await.unwrap();
    let (_, client) = aa.identity.configs().unwrap();
    let tcp = tokio::net::TcpStream::connect(("127.0.0.1", bb.port))
        .await
        .unwrap();
    let mut s = tokio_rustls::TlsConnector::from(client)
        .connect(
            rustls::pki_types::ServerName::try_from("localconnect.local")
                .unwrap()
                .to_owned(),
            tcp,
        )
        .await
        .unwrap();
    read_message(&mut s).await.unwrap();
    let src = a.path().join("attack.bin");
    std::fs::write(&src, b"unapproved").unwrap();
    let m = transfer::manifest(&src, uuid::Uuid::new_v4().to_string()).unwrap();
    write_message(&mut s, &Message::Bulk { manifest: m })
        .await
        .unwrap();
    assert!(
        matches!(read_message(&mut s).await.unwrap(),Message::Error{code,..} if code=="unpaired")
    );
    assert!(!b.path().join("received/attack.bin").exists());
    aa.shutdown().await;
    bb.shutdown().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dropped_connection_resumes_after_receiver_restart_and_control_is_independent() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let aa = Engine::start(cfg(a.path(), "A")).await.unwrap();
    let mut cb = cfg(b.path(), "B");
    let bb = Engine::start(cb.clone()).await.unwrap();
    cb.port = bb.port;
    aa.store.trust(&bb.identity.fingerprint, "B").unwrap();
    bb.store.trust(&aa.identity.fingerprint, "A").unwrap();
    let src = a.path().join("resume.bin");
    let data = vec![17; CHUNK as usize * 2 + 23];
    std::fs::write(&src, &data).unwrap();
    let m = transfer::manifest(&src, uuid::Uuid::new_v4().to_string()).unwrap();
    let address = format!("127.0.0.1:{}", bb.port);
    aa.store
        .register(
            &m,
            &bb.identity.fingerprint,
            "send",
            Some(src.to_str().unwrap()),
            &format!("{}:{}", bb.identity.fingerprint, m.id),
            Some(&address),
        )
        .unwrap();
    let (_, client) = aa.identity.configs().unwrap();
    let connector = tokio_rustls::TlsConnector::from(client);
    let server_name = rustls::pki_types::ServerName::try_from("localconnect.local")
        .unwrap()
        .to_owned();
    let mut bulk = connector
        .connect(
            server_name.clone(),
            tokio::net::TcpStream::connect(&address).await.unwrap(),
        )
        .await
        .unwrap();
    read_message(&mut bulk).await.unwrap();
    write_message(
        &mut bulk,
        &Message::Bulk {
            manifest: m.clone(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        read_message(&mut bulk).await.unwrap(),
        Message::Missing { .. }
    ));
    // Bulk socket deliberately stalls. Independent control must still respond.
    let mut control = connector
        .connect(
            server_name,
            tokio::net::TcpStream::connect(&address).await.unwrap(),
        )
        .await
        .unwrap();
    read_message(&mut control).await.unwrap();
    write_message(&mut control, &Message::Control)
        .await
        .unwrap();
    read_message(&mut control).await.unwrap();
    write_message(&mut control, &Message::Ping { nonce: 123 })
        .await
        .unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(1), read_message(&mut control))
            .await
            .unwrap()
            .unwrap(),
        Message::Pong { nonce: 123 }
    ));
    drop(control);
    write_chunk(&mut bulk, 0, &data[..CHUNK as usize])
        .await
        .unwrap();
    assert!(matches!(
        read_message(&mut bulk).await.unwrap(),
        Message::Ack { index: 0, .. }
    ));
    drop(bulk);
    bb.shutdown().await;
    let restarted = Engine::start(cb).await.unwrap();
    aa.resume(m.id.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if aa.store.transfer_status(&m.id).unwrap() == "complete" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await
        }
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(b.path().join("received/resume.bin")).unwrap(),
        data
    );
    assert_eq!(restarted.store.indexes(&m.id).unwrap(), vec![0, 1, 2]);
    aa.shutdown().await;
    restarted.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_cancels_pending_pairing_without_waiting_for_consent_deadline() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let aa = Engine::start(cfg(a.path(), "A")).await.unwrap();
    let bb = Engine::start(cfg(b.path(), "B")).await.unwrap();
    let mut events = bb.events.subscribe();
    let initiator = aa.clone();
    let addr = format!("127.0.0.1:{}", bb.port);
    let fingerprint = bb.identity.fingerprint.clone();
    let pairing = tokio::spawn(async move { initiator.pair(addr, fingerprint).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if events.recv().await.unwrap().kind == "pair_request" {
                break;
            }
        }
    })
    .await
    .unwrap();
    aa.shutdown().await;
    let error = tokio::time::timeout(Duration::from_secs(1), pairing)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("cancelled"));
    assert!(!aa.store.trusted(&bb.identity.fingerprint).unwrap());
    bb.shutdown().await;
}

#[tokio::test]
async fn transfer_policy_survives_restart() {
    use localconnect_core::engine::TransferLimits;
    let d = tempfile::tempdir().unwrap();
    let config = cfg(d.path(), "policy");
    let engine = Engine::start(config.clone()).await.unwrap();
    engine
        .configure_transfer(TransferLimits {
            in_flight: 2,
            max_file_bytes: 123,
            max_reserved_bytes: 456,
        })
        .await
        .unwrap();
    engine.shutdown().await;
    let restarted = Engine::start(config).await.unwrap();
    let saved = restarted.transfer_limits();
    restarted.shutdown().await;
    assert_eq!(
        (
            saved.in_flight,
            saved.max_file_bytes,
            saved.max_reserved_bytes
        ),
        (2, 123, 456)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn code_pairing_transfers_with_session_trust_and_cannot_reuse_code() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let aa = Engine::start(cfg(a.path(), "Phone")).await.unwrap();
    let bb = Engine::start(cfg(b.path(), "PC")).await.unwrap();
    let display: serde_json::Value =
        serde_json::from_str(&bb.begin_pairing_code(false).await.unwrap()).unwrap();
    let code = display["code"].as_str().unwrap().to_owned();
    let address = format!("127.0.0.1:{}", bb.port);
    let peer = aa
        .pair_with_code(address.clone(), code.clone(), false)
        .await
        .unwrap();
    assert!(!aa.store.trusted(&bb.identity.fingerprint).unwrap());
    assert!(!bb.store.trusted(&aa.identity.fingerprint).unwrap());
    let source = a.path().join("code-paired.bin");
    let bytes = vec![19u8; 128 * 1024];
    std::fs::write(&source, &bytes).unwrap();
    let id = aa
        .send(source.to_str().unwrap().into(), peer.clone())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if bb
                .store
                .transfer_status(&id)
                .is_ok_and(|status| status == "complete")
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(b.path().join("received/code-paired.bin")).unwrap(),
        bytes
    );
    assert!(aa.pair_with_code(address, code, false).await.is_err());
    aa.forget(peer.fingerprint.clone()).await.unwrap();
    assert!(aa.start_control(peer.clone()).await.is_err());
    aa.shutdown().await;
    bb.shutdown().await;
    let aa = Engine::start(cfg(a.path(), "Phone")).await.unwrap();
    let bb = Engine::start(cfg(b.path(), "PC")).await.unwrap();
    assert!(aa.start_control(peer).await.is_err());
    assert!(!bb.store.trusted(&aa.identity.fingerprint).unwrap());
    aa.shutdown().await;
    bb.shutdown().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn code_pairing_rejects_wrong_codes_and_exhausted_window_without_trust() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let aa = Engine::start(cfg(a.path(), "Phone")).await.unwrap();
    let bb = Engine::start(cfg(b.path(), "PC")).await.unwrap();
    let display: serde_json::Value =
        serde_json::from_str(&bb.begin_pairing_code(false).await.unwrap()).unwrap();
    let code = display["code"].as_str().unwrap().to_owned();
    let wrong = if code == "000000" { "000001" } else { "000000" };
    let address = format!("127.0.0.1:{}", bb.port);
    for _ in 0..3 {
        assert!(aa
            .pair_with_code(address.clone(), wrong.into(), false)
            .await
            .is_err());
    }
    assert!(aa
        .pair_with_code(address.clone(), code, false)
        .await
        .is_err());
    assert!(!aa.store.trusted(&bb.identity.fingerprint).unwrap());
    assert!(!bb.store.trusted(&aa.identity.fingerprint).unwrap());
    let peer = aa.probe(address).await.unwrap();
    assert!(aa.start_control(peer).await.is_err());
    aa.shutdown().await;
    bb.shutdown().await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn code_pairing_remember_option_survives_restart_and_forget_revokes() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let aa = Engine::start(cfg(a.path(), "Phone")).await.unwrap();
    let bb = Engine::start(cfg(b.path(), "PC")).await.unwrap();
    let display: serde_json::Value =
        serde_json::from_str(&bb.begin_pairing_code(true).await.unwrap()).unwrap();
    aa.pair_with_code(
        format!("127.0.0.1:{}", bb.port),
        display["code"].as_str().unwrap().into(),
        true,
    )
    .await
    .unwrap();
    assert!(aa.store.trusted(&bb.identity.fingerprint).unwrap());
    assert!(bb.store.trusted(&aa.identity.fingerprint).unwrap());
    aa.shutdown().await;
    bb.shutdown().await;
    let aa = Engine::start(cfg(a.path(), "Phone")).await.unwrap();
    let bb = Engine::start(cfg(b.path(), "PC")).await.unwrap();
    let peer = aa.probe(format!("127.0.0.1:{}", bb.port)).await.unwrap();
    let mut events = aa.events.subscribe();
    aa.start_control(peer.clone()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.kind == "connection" {
                let payload: serde_json::Value = serde_json::from_str(&event.payload).unwrap();
                if payload["state"] == "connected" {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    aa.forget(peer.fingerprint.clone()).await.unwrap();
    assert!(!aa.store.trusted(&peer.fingerprint).unwrap());
    assert!(aa.start_control(peer).await.is_err());
    aa.shutdown().await;
    bb.shutdown().await;
}
