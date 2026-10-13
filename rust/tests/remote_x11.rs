//! Run only on the isolated synthetic display created by tests/run_x11.py.
use localconnect_core::{
    engine::{Config, Engine, Peer},
    protocol::*,
    remote::RemoteMessage,
};
use std::time::Duration;
fn config(path: &std::path::Path, name: &str) -> Config {
    Config {
        data_dir: path.join("state").display().to_string(),
        receive_dir: path.join("received").display().to_string(),
        name: name.into(),
        port: 0,
        in_flight: 4,
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires isolated authenticated Xvfb; use tests/run_x11.py"]
async fn approved_stream_pointer_and_stop_cleanup() {
    assert_eq!(std::env::var("LOCALCONNECT_ISOLATED_X11").unwrap(), "1");
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let viewer = Engine::start(config(a.path(), "viewer")).await.unwrap();
    let source = Engine::start(config(b.path(), "controlled")).await.unwrap();
    viewer
        .store
        .trust(&source.identity.fingerprint, "test fixture")
        .unwrap();
    source
        .store
        .trust(&viewer.identity.fingerprint, "test fixture")
        .unwrap();
    let mut requests = source.events.subscribe();
    let owner = source.clone();
    let approval = tokio::spawn(async move {
        loop {
            let event = requests.recv().await.unwrap();
            if event.kind == "remote_request" {
                let data: serde_json::Value = serde_json::from_str(&event.payload).unwrap();
                owner
                    .approve_remote(data["request"].as_str().unwrap().into(), true)
                    .await
                    .unwrap();
                break;
            }
        }
    });
    let peer = Peer {
        fingerprint: source.identity.fingerprint.clone(),
        name: "controlled".into(),
        address: format!("127.0.0.1:{}", source.port),
    };
    let mut frames = viewer.events.subscribe();
    let mut ended = source.events.subscribe();
    let session = viewer.request_remote(peer, true).await.unwrap();
    approval.await.unwrap();
    let png = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let event = frames.recv().await.unwrap();
            if event.kind == "remote_frame" {
                let data: serde_json::Value = serde_json::from_str(&event.payload).unwrap();
                break std::fs::read(data["path"].as_str().unwrap()).unwrap();
            }
        }
    })
    .await
    .unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(u32::from_be_bytes(png[16..20].try_into().unwrap()), 800);
    viewer
        .send_remote_input(
            session.id.clone(),
            serde_json::to_string(&RemoteMessage::Pointer {
                id: session.id.clone(),
                x: 0.25,
                y: 0.5,
            })
            .unwrap(),
            session.geometry.clone(),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    #[cfg(all(feature = "desktop-remote", target_os = "linux"))]
    {
        use enigo::{Enigo, Mouse, Settings};
        let input = Enigo::new(&Settings::default()).unwrap();
        assert_eq!(input.location().unwrap(), (200, 300));
    }
    viewer
        .send_remote_input(
            session.id.clone(),
            serde_json::to_string(&RemoteMessage::Key {
                id: session.id.clone(),
                code: 0x110005,
                down: true,
            })
            .unwrap(),
            session.geometry.clone(),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    #[cfg(target_os = "linux")]
    {
        use x11rb::{
            connection::Connection,
            protocol::xproto::{ConnectionExt, KeyButMask},
        };
        let (connection, screen) = x11rb::connect(None).unwrap();
        let root = connection.setup().roots[screen].root;
        assert!(connection
            .query_pointer(root)
            .unwrap()
            .reply()
            .unwrap()
            .mask
            .contains(KeyButMask::SHIFT));
    }
    source.stop_remote(session.id.clone()).await.unwrap();
    #[cfg(target_os = "linux")]
    {
        use x11rb::{
            connection::Connection,
            protocol::xproto::{ConnectionExt, KeyButMask},
        };
        let (connection, screen) = x11rb::connect(None).unwrap();
        let root = connection.setup().roots[screen].root;
        assert!(!connection
            .query_pointer(root)
            .unwrap()
            .reply()
            .unwrap()
            .mask
            .contains(KeyButMask::SHIFT));
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let event = ended.recv().await.unwrap();
            if event.kind == "remote_ended" {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(viewer
        .send_remote_input(
            session.id.clone(),
            serde_json::to_string(&RemoteMessage::Pointer {
                id: session.id.clone(),
                x: 0.5,
                y: 0.5
            })
            .unwrap(),
            session.geometry.clone()
        )
        .await
        .is_err());
    viewer.shutdown().await;
    source.shutdown().await;
}
#[tokio::test(flavor = "multi_thread")]
async fn unpaired_remote_and_video_are_rejected() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let attacker = Engine::start(config(a.path(), "unknown")).await.unwrap();
    let source = Engine::start(config(b.path(), "source")).await.unwrap();
    for message in [
        Message::RemoteRequest {
            request: uuid::Uuid::new_v4().to_string(),
            input: true,
        },
        Message::RemoteVideo {
            session: uuid::Uuid::new_v4().to_string(),
        },
    ] {
        let (_, client) = attacker.identity.configs().unwrap();
        let tcp = tokio::net::TcpStream::connect(("127.0.0.1", source.port))
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
        write_message(&mut socket, &message).await.unwrap();
        assert!(
            matches!(read_message(&mut socket).await.unwrap(),Message::Error{code,..}if code=="unpaired")
        );
    }
    attacker.shutdown().await;
    source.shutdown().await;
}
