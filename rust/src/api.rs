use crate::engine::{Config, Engine, Event, Peer, TransferLimits};
use crate::frb_generated::StreamSink;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex;
static GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static ENGINE: OnceLock<Mutex<Option<Arc<Engine>>>> = OnceLock::new();
fn cell() -> &'static Mutex<Option<Arc<Engine>>> {
    ENGINE.get_or_init(|| Mutex::new(None))
}
async fn engine() -> anyhow::Result<Arc<Engine>> {
    cell()
        .lock()
        .await
        .clone()
        .ok_or_else(|| anyhow::anyhow!("initialize LocalConnect first"))
}
pub async fn initialize(
    data_dir: String,
    receive_dir: String,
    name: String,
    port: u16,
) -> anyhow::Result<String> {
    let mut slot = cell().lock().await;
    anyhow::ensure!(slot.is_none(), "already initialized");
    let e = Engine::start(Config {
        data_dir,
        receive_dir,
        name,
        port,
        in_flight: 4,
    })
    .await?;
    let generation = GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    let info = serde_json::json!({"fingerprint":e.identity.fingerprint,"port":e.port,"generation":generation}).to_string();
    *slot = Some(e);
    Ok(info)
}
pub async fn events(sink: StreamSink<Event>) -> anyhow::Result<()> {
    let e = engine().await?;
    let (mut receiver, snapshot) = e.subscribe_events();
    for event in snapshot {
        if sink.add(event).is_err() {
            return Ok(());
        }
    }
    loop {
        let stopped = e.cancellation();
        let received = tokio::select! {_=stopped.cancelled()=>break,r=receiver.recv()=>r};
        match received {
            Ok(event) => {
                if sink.add(event).is_err() {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                let (new_receiver, snapshot) = e.subscribe_events();
                receiver = new_receiver;
                for event in snapshot {
                    if sink.add(event).is_err() {
                        return Ok(());
                    }
                }
                let _ = sink.add(Event {
                    kind: "resync".into(),
                    payload: "{}".into(),
                });
            }
            Err(_) => break,
        }
    }
    Ok(())
}
pub async fn probe(address: String) -> anyhow::Result<Peer> {
    engine().await?.probe(address).await
}
pub async fn pair(address: String, fingerprint: String) -> anyhow::Result<Peer> {
    engine().await?.pair(address, fingerprint).await
}
pub async fn approve_pair(fingerprint: String, allow: bool) -> anyhow::Result<()> {
    engine().await?.approve(fingerprint, allow).await
}
pub async fn connect(peer: Peer) -> anyhow::Result<()> {
    engine().await?.start_control(peer).await
}
pub async fn send_file(path: String, peer: Peer) -> anyhow::Result<String> {
    engine().await?.send(path, peer).await
}
pub async fn resume_transfer(id: String) -> anyhow::Result<()> {
    engine().await?.resume(id).await
}
pub async fn cancel_transfer(id: String) -> anyhow::Result<()> {
    engine().await?.cancel(id).await
}
pub async fn trusted_peers() -> anyhow::Result<String> {
    engine().await?.peers().await
}
pub async fn transfer_history() -> anyhow::Result<String> {
    engine().await?.history().await
}
pub async fn forget_peer(fingerprint: String) -> anyhow::Result<()> {
    engine().await?.forget(fingerprint).await
}
pub fn capabilities() -> String {
    crate::remote::capabilities()
}
pub async fn shutdown() -> anyhow::Result<()> {
    if let Some(e) = cell().lock().await.take() {
        e.shutdown().await;
    }
    Ok(())
}

pub async fn update_transfer_address(id: String, address: String) -> anyhow::Result<()> {
    engine().await?.relocate(id, address).await
}
pub async fn transfer_limits() -> anyhow::Result<TransferLimits> {
    Ok(engine().await?.transfer_limits())
}
pub async fn configure_transfer(limits: TransferLimits) -> anyhow::Result<()> {
    engine().await?.configure_transfer(limits).await
}
pub async fn cleanup_transfer(id: String) -> anyhow::Result<()> {
    engine().await?.cleanup_transfer(id).await
}
pub async fn request_remote(
    peer: Peer,
    input: bool,
) -> anyhow::Result<crate::remote_engine::RemoteView> {
    engine().await?.request_remote(peer, input).await
}
pub async fn approve_remote(request: String, allow: bool) -> anyhow::Result<()> {
    engine().await?.approve_remote(request, allow).await
}
pub async fn send_remote_input(
    id: String,
    event: String,
    geometry: crate::remote::Geometry,
) -> anyhow::Result<()> {
    engine().await?.send_remote_input(id, event, geometry).await
}
pub async fn stop_remote(id: String) -> anyhow::Result<()> {
    engine().await?.stop_remote(id).await
}
pub async fn transfer_snapshot() -> anyhow::Result<Vec<crate::engine::TransferProgress>> {
    let store = engine().await?.store.clone();
    crate::engine::blocking(move || store.snapshot()).await
}
pub fn explain_error(message: String) -> crate::errors::FailureInfo {
    crate::errors::from_error(&anyhow::anyhow!(message))
}
pub async fn set_native_platform(
    geometry: crate::remote::Geometry,
    input: bool,
) -> anyhow::Result<()> {
    engine().await?.set_native_platform(geometry, input)
}
pub async fn publish_native_frame(
    id: String,
    path: String,
    geometry: crate::remote::Geometry,
    width: u32,
    height: u32,
) -> anyhow::Result<()> {
    engine()
        .await?
        .publish_native_frame(id, path, geometry, width, height)
        .await
}
#[cfg(any(target_os = "android", test))]
#[flutter_rust_bridge::frb(ignore)]
pub(crate) async fn native_shutdown(expected: u64) {
    let mut slot = cell().lock().await;
    if GENERATION.load(std::sync::atomic::Ordering::SeqCst) != expected {
        return;
    }
    if let Some(engine) = slot.take() {
        engine.shutdown().await;
    }
}
#[cfg(any(target_os = "android", test))]
#[flutter_rust_bridge::frb(ignore)]
pub(crate) async fn native_stop_session(expected: u64, id: String) {
    let slot = cell().lock().await;
    if GENERATION.load(std::sync::atomic::Ordering::SeqCst) != expected {
        return;
    }
    if let Some(engine) = slot.as_ref() {
        let _ = engine.stop_remote(id).await;
    }
}

pub async fn begin_pairing_code(remember: bool) -> anyhow::Result<String> {
    engine().await?.begin_pairing_code(remember).await
}
pub async fn cancel_pairing_code() -> anyhow::Result<()> {
    engine().await?.cancel_pairing_code().await;
    Ok(())
}
pub async fn pair_with_code(address: String, code: String, remember: bool) -> anyhow::Result<Peer> {
    engine()
        .await?
        .pair_with_code(address, code, remember)
        .await
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn stale_native_callback_cannot_stop_restarted_engine() {
        let dir = tempfile::tempdir().unwrap();
        let start = || {
            super::initialize(
                dir.path().join("state").display().to_string(),
                dir.path().join("received").display().to_string(),
                "lifecycle".into(),
                0,
            )
        };
        let first: serde_json::Value = serde_json::from_str(&start().await.unwrap()).unwrap();
        super::shutdown().await.unwrap();
        let second: serde_json::Value = serde_json::from_str(&start().await.unwrap()).unwrap();
        super::native_shutdown(first["generation"].as_u64().unwrap()).await;
        super::native_stop_session(
            first["generation"].as_u64().unwrap(),
            uuid::Uuid::new_v4().to_string(),
        )
        .await;
        let survived = super::engine().await.is_ok();
        super::shutdown().await.unwrap();
        assert!(survived);
        assert_ne!(first["generation"], second["generation"]);
    }
}
