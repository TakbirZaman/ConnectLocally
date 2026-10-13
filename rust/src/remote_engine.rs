use crate::{
    desktop,
    engine::{blocking, Engine, Event, Peer},
    protocol::*,
    remote::{Geometry, RemoteMessage},
    remote_session::Sessions,
};
use anyhow::{ensure, Result};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot},
    time::timeout,
};
use tokio_util::sync::CancellationToken;
pub(crate) async fn wait_input_done(mut done: tokio::sync::watch::Receiver<bool>) -> Result<()> {
    timeout(Duration::from_secs(3), async {
        while !*done.borrow() {
            done.changed().await?;
        }
        Ok::<(), anyhow::Error>(())
    })
    .await??;
    Ok(())
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RemoteView {
    pub id: String,
    pub geometry: Geometry,
    pub input: bool,
    pub platform: String,
}
struct Outgoing {
    peer: String,
    input: bool,
    sender: mpsc::Sender<(RemoteMessage, Geometry)>,
    stop: CancellationToken,
}
#[derive(Default)]
pub(crate) struct RemoteState {
    sessions: Mutex<Sessions>,
    pending: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    outgoing: Mutex<HashMap<String, Outgoing>>,
    video: Mutex<HashSet<String>>,
    view_busy: Arc<std::sync::atomic::AtomicBool>,
    input_done: Mutex<HashMap<String, tokio::sync::watch::Receiver<bool>>>,
    native: Mutex<Option<(Geometry, bool)>>,
    platform_frames:
        Mutex<HashMap<String, tokio::sync::watch::Sender<Option<Arc<desktop::Frame>>>>>,
}
struct ViewReservation(Arc<std::sync::atomic::AtomicBool>);
impl Drop for ViewReservation {
    fn drop(&mut self) {
        self.0.store(false, std::sync::atomic::Ordering::Release);
    }
}
impl RemoteState {
    pub fn snapshot(&self) -> Vec<Event> {
        let mut values = self.sessions.lock().unwrap().snapshot();
        values.extend(self.outgoing.lock().unwrap().iter().filter(|(_,s)|!s.stop.is_cancelled()).map(|(id,s)|serde_json::json!({"id":id,"role":"viewer","peer":s.peer,"input":s.input})));
        let mut events = vec![Event {
            kind: "remote_reset".into(),
            payload: "{}".into(),
        }];
        events.extend(values.into_iter().map(|value| Event {
            kind: "remote_active".into(),
            payload: value.to_string(),
        }));
        events
    }
    pub fn revoke(&self, peer: &str) -> Vec<tokio::sync::watch::Receiver<bool>> {
        let ids = self.sessions.lock().unwrap().revoke(peer);
        for id in &ids {
            self.pending.lock().unwrap().remove(id);
            self.platform_frames.lock().unwrap().remove(id);
        }
        for client in self.outgoing.lock().unwrap().values() {
            if client.peer == peer {
                client.stop.cancel();
            }
        }
        let mut completions = self.input_done.lock().unwrap();
        ids.into_iter()
            .filter_map(|id| completions.remove(&id))
            .collect()
    }
    pub fn stop_all(&self) -> Vec<tokio::sync::watch::Receiver<bool>> {
        let input_cleanup = self
            .input_done
            .lock()
            .unwrap()
            .drain()
            .map(|(_, done)| done)
            .collect();
        self.pending.lock().unwrap().clear();
        self.platform_frames.lock().unwrap().clear();
        *self.sessions.lock().unwrap() = Sessions::default();
        for (_, client) in self.outgoing.lock().unwrap().drain() {
            client.stop.cancel();
        }
        input_cleanup
    }
}
struct Lease {
    state: Arc<RemoteState>,
    id: String,
    events: tokio::sync::broadcast::Sender<Event>,
    controlled: bool,
    _reservation: Option<ViewReservation>,
    frames: Option<std::path::PathBuf>,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if self.controlled {
            self.state.sessions.lock().unwrap().end(&self.id);
            self.state.platform_frames.lock().unwrap().remove(&self.id);
        } else if let Some(c) = self.state.outgoing.lock().unwrap().remove(&self.id) {
            c.stop.cancel();
        }
        let _ = self.events.send(Event {
            kind: "remote_ended".into(),
            payload: serde_json::json!({"id":self.id}).to_string(),
        });
        if let Some(directory) = self.frames.clone() {
            let id = self.id.clone();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn_blocking(move || {
                    for slot in 0..2 {
                        let _ = std::fs::remove_file(directory.join(format!("{id}-{slot}.png")));
                    }
                });
            }
        }
    }
}
struct VideoLease {
    state: Arc<RemoteState>,
    id: String,
    stop: CancellationToken,
}
impl Drop for VideoLease {
    fn drop(&mut self) {
        self.state.video.lock().unwrap().remove(&self.id);
        self.stop.cancel();
    }
}
impl Engine {
    pub fn set_native_platform(&self, geometry: Geometry, input: bool) -> Result<()> {
        ensure!(
            cfg!(target_os = "android"),
            "native platform bridge is Android-only"
        );
        geometry.validate()?;
        *self.remote.native.lock().unwrap() = Some((geometry, input));
        Ok(())
    }
    pub async fn publish_native_frame(
        &self,
        id: String,
        path: String,
        geometry: Geometry,
        width: u32,
        height: u32,
    ) -> Result<()> {
        ensure!(
            cfg!(target_os = "android"),
            "native frames are Android-only"
        );
        geometry.validate()?;
        let sender = self
            .remote
            .platform_frames
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("native screen session not active"))?;
        self.remote
            .sessions
            .lock()
            .unwrap()
            .update_geometry(&id, geometry.clone())?;
        let allowed = std::path::PathBuf::from(&self.config.data_dir)
            .parent()
            .ok_or_else(|| anyhow::anyhow!("application storage unavailable"))?
            .join("screens");
        let bytes = blocking(move || {
            let path = std::path::PathBuf::from(path);
            ensure!(
                std::fs::symlink_metadata(&path)?.is_file(),
                "capture frame must be a regular file"
            );
            let path = std::fs::canonicalize(path)?;
            let allowed = std::fs::canonicalize(allowed)?;
            ensure!(
                path.parent() == Some(allowed.as_path()),
                "capture frame outside private screen directory"
            );
            ensure!(
                std::fs::metadata(&path)?.len() <= 8 * 1024 * 1024,
                "native frame too large"
            );
            use std::io::Read;
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(8 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            ensure!(bytes.len() <= 8 * 1024 * 1024, "native frame too large");
            Ok(bytes)
        })
        .await?;
        ensure!(
            width > 0
                && height > 0
                && width <= 1920
                && height <= 1920
                && width as u64 * height as u64 <= 1920 * 1080,
            "native frame geometry exceeds limit"
        );
        ensure!(
            bytes.len() >= 24
                && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
                && u32::from_be_bytes(bytes[16..20].try_into()?) == width
                && u32::from_be_bytes(bytes[20..24].try_into()?) == height,
            "native PNG dimensions mismatch"
        );
        sender.send_replace(Some(Arc::new(desktop::Frame {
            bytes,
            width,
            height,
            geometry,
            timestamp_us: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_micros() as u64,
        })));
        Ok(())
    }
    pub async fn approve_remote(&self, request: String, allow: bool) -> Result<()> {
        let sender = self
            .remote
            .pending
            .lock()
            .unwrap()
            .remove(&request)
            .ok_or_else(|| anyhow::anyhow!("remote request expired"))?;
        sender
            .send(allow)
            .map_err(|_| anyhow::anyhow!("requesting device disconnected"))
    }
    pub async fn stop_remote(&self, id: String) -> Result<()> {
        let completion = self.remote.input_done.lock().unwrap().get(&id).cloned();
        self.remote.sessions.lock().unwrap().end(&id);
        if let Some(client) = self.remote.outgoing.lock().unwrap().get(&id) {
            client.stop.cancel();
        }
        if let Some(completion) = completion {
            wait_input_done(completion).await?;
        }
        self.remote.input_done.lock().unwrap().remove(&id);
        Ok(())
    }
    pub async fn send_remote_input(
        &self,
        id: String,
        event: String,
        geometry: Geometry,
    ) -> Result<()> {
        ensure!(event.len() <= 8192, "remote input metadata too large");
        let event: RemoteMessage = serde_json::from_str(&event)?;
        let sender = self
            .remote
            .outgoing
            .lock()
            .unwrap()
            .get(&id)
            .map(|c| c.sender.clone())
            .ok_or_else(|| anyhow::anyhow!("remote session not active"))?;
        sender
            .try_send((event, geometry))
            .map_err(|_| anyhow::anyhow!("remote input queue full or disconnected"))
    }
    pub async fn request_remote(self: &Arc<Self>, peer: Peer, input: bool) -> Result<RemoteView> {
        let stop = self.cancellation();
        tokio::select! {biased;_=stop.cancelled()=>anyhow::bail!("stopped"),result=self.request_remote_inner(peer,input)=>result}
    }
    async fn request_remote_inner(self: &Arc<Self>, peer: Peer, input: bool) -> Result<RemoteView> {
        ensure!(
            self.trusted(&peer.fingerprint).await?,
            "pair before requesting remote control"
        );
        ensure!(
            self.remote
                .view_busy
                .compare_exchange(
                    false,
                    true,
                    std::sync::atomic::Ordering::AcqRel,
                    std::sync::atomic::Ordering::Acquire
                )
                .is_ok(),
            "another viewed session/request is active"
        );
        let reservation = ViewReservation(self.remote.view_busy.clone());
        let (mut socket, _) = self.connect(&peer.address, Some(&peer.fingerprint)).await?;
        write_message(
            &mut socket,
            &Message::RemoteRequest {
                request: uuid::Uuid::new_v4().to_string(),
                input,
            },
        )
        .await?;
        let shutdown = self.cancellation();
        let message = tokio::select! {_=shutdown.cancelled()=>anyhow::bail!("stopped"),r=timeout(Duration::from_secs(180),read_message(&mut socket))=>r??};
        let view = match message {
            Message::RemoteApproved {
                session,
                geometry,
                input,
                platform,
            } => {
                geometry.validate()?;
                ensure!(
                    uuid::Uuid::parse_str(&session)?.to_string() == session,
                    "session ID must be canonical UUID"
                );
                RemoteView {
                    id: session,
                    geometry,
                    input,
                    platform,
                }
            }
            Message::RemoteDenied { reason }
            | Message::Error {
                message: reason, ..
            } => anyhow::bail!("remote request denied: {reason}"),
            _ => anyhow::bail!("remote approval expected"),
        };
        let (sender, mut commands) = mpsc::channel(128);
        let stop = self.cancellation().child_token();
        self.remote.outgoing.lock().unwrap().insert(
            view.id.clone(),
            Outgoing {
                peer: peer.fingerprint.clone(),
                input: view.input,
                sender,
                stop: stop.clone(),
            },
        );
        self.emit("remote_active",serde_json::json!({"id":view.id,"role":"viewer","peer":peer.fingerprint,"input":view.input}));
        let engine = self.clone();
        let id = view.id.clone();
        let token = stop.clone();
        let peer_fp = peer.fingerprint.clone();
        let lease = Lease {
            state: self.remote.clone(),
            id: id.clone(),
            events: self.events.clone(),
            controlled: false,
            _reservation: Some(reservation),
            frames: Some(std::path::PathBuf::from(&self.config.data_dir).join("remote-frames")),
        };
        self.tasks.lock().await.push(tokio::spawn(async move {
            let _lease=lease;
            let result=async {
                let (mut reader,mut writer)=tokio::io::split(socket);
                let (pong_tx,mut pongs)=mpsc::channel(4);
                let read=async {loop {match read_message(&mut reader).await?{Message::Pong{nonce}=>pong_tx.send(nonce).await?,Message::Error{message,..}=>anyhow::bail!("{message}"),_=>anyhow::bail!("remote control connection closed")}} #[allow(unreachable_code)] Ok::<(),anyhow::Error>(())};
                let write=async {
                    let mut ticks=tokio::time::interval(Duration::from_secs(15));ticks.tick().await;
                    loop {tokio::select!{
                        _=ticks.tick()=>{ensure!(engine.trusted(&peer_fp).await?,"trust revoked");let nonce=rand::random();write_message(&mut writer,&Message::Ping{nonce}).await?;ensure!(timeout(Duration::from_secs(10),pongs.recv()).await?==Some(nonce),"remote heartbeat failed");},
                        command=commands.recv()=>{let Some((event,geometry))=command else{break};write_message(&mut writer,&Message::RemoteInput{event,geometry}).await?;}
                    }}Ok::<(),anyhow::Error>(())
                };
                tokio::select!{_=token.cancelled()=>Ok(()),result=read=>result,result=write=>result}
            }.await;
            if let Err(error)=result{engine.emit("error",serde_json::json!({"code":"remote_failed","id":id,"message":error.to_string()}));}
        }));
        let engine = self.clone();
        let video_view = view.clone();
        self.tasks.lock().await.push(tokio::spawn(async move{
            let result=tokio::select!{_=stop.cancelled()=>Ok(()),result=engine.receive_video(peer,video_view.clone())=>result};
            stop.cancel();
            if let Err(error)=result{engine.emit("error",serde_json::json!({"code":"video_failed","id":video_view.id,"message":error.to_string()}));}
        }));
        Ok(view)
    }
    async fn receive_video(&self, peer: Peer, view: RemoteView) -> Result<()> {
        let stop = self
            .remote
            .outgoing
            .lock()
            .unwrap()
            .get(&view.id)
            .map(|client| client.stop.clone())
            .ok_or_else(|| anyhow::anyhow!("viewer already stopped"))?;
        let (mut socket, _) = self.connect(&peer.address, Some(&peer.fingerprint)).await?;
        write_message(
            &mut socket,
            &Message::RemoteVideo {
                session: view.id.clone(),
            },
        )
        .await?;
        ensure!(
            matches!(read_message(&mut socket).await?, Message::Ready),
            "video channel not approved"
        );
        let mut previous = None;
        loop {
            ensure!(self.trusted(&peer.fingerprint).await?, "trust revoked");
            let (sequence, width, height, geometry, bytes) =
                match timeout(Duration::from_secs(10), read_message(&mut socket)).await?? {
                    Message::VideoFrame {
                        session,
                        sequence,
                        timestamp_us: _,
                        codec,
                        keyframe,
                        width,
                        height,
                        geometry,
                        bytes,
                    } => {
                        ensure!(session == view.id, "frame belongs to another session");
                        ensure!(
                            codec == "png" && keyframe,
                            "unsupported screen codec/frame dependency"
                        );
                        (sequence, width, height, geometry, bytes)
                    }
                    _ => anyhow::bail!("invalid video metadata"),
                };
            geometry.validate()?;
            ensure!(
                previous.is_none_or(|n| sequence > n),
                "stale video sequence"
            );
            previous = Some(sequence);
            let data = timeout(
                Duration::from_secs(5),
                read_video(&mut socket, sequence, bytes, width, height),
            )
            .await??;
            let path = std::path::PathBuf::from(&self.config.data_dir)
                .join("remote-frames")
                .join(format!("{}-{}.png", view.id, sequence % 2));
            let output = path.clone();
            let write_stop = stop.clone();
            blocking(move || {
                ensure!(!write_stop.is_cancelled(), "viewer stopped");
                std::fs::create_dir_all(output.parent().unwrap())?;
                let temporary = output.with_extension("tmp");
                std::fs::write(&temporary, data)?;
                if write_stop.is_cancelled() {
                    let _ = std::fs::remove_file(&temporary);
                    anyhow::bail!("viewer stopped");
                }
                std::fs::rename(temporary, &output)?;
                if write_stop.is_cancelled() {
                    let _ = std::fs::remove_file(output);
                    anyhow::bail!("viewer stopped");
                }
                Ok(())
            })
            .await?;
            self.emit("remote_frame",serde_json::json!({"id":view.id,"sequence":sequence,"path":path,"width":width,"height":height,"geometry":geometry}));
        }
    }
    pub(crate) async fn serve_remote(
        self: &Arc<Self>,
        mut socket: tokio_rustls::server::TlsStream<TcpStream>,
        peer: String,
        first: Message,
    ) -> Result<()> {
        match first {
            Message::RemoteRequest { request, input } => {
                let native = self.remote.native.lock().unwrap().clone();
                let wayland = crate::wayland::session_is_wayland() && crate::wayland::compiled();
                if !desktop::supported() && native.is_none() && !wayland {
                    write_message(
                        &mut socket,
                        &Message::RemoteDenied {
                            reason: "capture/input unavailable on this platform or display session"
                                .into(),
                        },
                    )
                    .await?;
                    return Ok(());
                }
                self.remote
                    .input_done
                    .lock()
                    .unwrap()
                    .retain(|_, done| !*done.borrow());
                ensure!(
                    self.remote.input_done.lock().unwrap().is_empty(),
                    "previous input worker is still stopping"
                );
                let geometry = if let Some((geometry, _)) = &native {
                    geometry.clone()
                } else if wayland {
                    Geometry {
                        width: 1,
                        height: 1,
                        rotation: 0,
                        scale: 1.0,
                    }
                } else {
                    match blocking(desktop::geometry).await {
                        Ok(geometry) => geometry,
                        Err(error) => {
                            write_message(
                                &mut socket,
                                &Message::RemoteDenied {
                                    reason: error.to_string(),
                                },
                            )
                            .await?;
                            return Ok(());
                        }
                    }
                };
                let input = input && native.as_ref().is_none_or(|(_, available)| *available);
                self.remote.sessions.lock().unwrap().request(
                    request.clone(),
                    peer.clone(),
                    geometry.clone(),
                    input,
                )?;
                let (sender, decision) = oneshot::channel();
                self.remote
                    .pending
                    .lock()
                    .unwrap()
                    .insert(request.clone(), sender);
                self.emit(
                    "remote_request",
                    serde_json::json!({"request":request,"peer":peer,"input":input}),
                );
                let allow = matches!(
                    timeout(Duration::from_secs(90), decision).await,
                    Ok(Ok(true))
                );
                self.remote.pending.lock().unwrap().remove(&request);
                if let Some((geometry, input)) = self.remote.native.lock().unwrap().clone() {
                    self.remote
                        .sessions
                        .lock()
                        .unwrap()
                        .update_pending(&request, geometry, input)?;
                }
                let session = self
                    .remote
                    .sessions
                    .lock()
                    .unwrap()
                    .decide(&request, allow)?;
                let Some(id) = session else {
                    write_message(
                        &mut socket,
                        &Message::RemoteDenied {
                            reason: "not approved or expired".into(),
                        },
                    )
                    .await?;
                    return Ok(());
                };
                let (mut geometry, mut input) = {
                    let sessions = self.remote.sessions.lock().unwrap();
                    let session = sessions.authorize(&id, &peer)?;
                    (session.geometry.clone(), session.input)
                };
                let lease = Lease {
                    state: self.remote.clone(),
                    id: id.clone(),
                    events: self.events.clone(),
                    controlled: true,
                    _reservation: None,
                    frames: None,
                };
                let token = self
                    .remote
                    .sessions
                    .lock()
                    .unwrap()
                    .authorize(&id, &peer)?
                    .stop
                    .clone();
                let portal = if wayland {
                    match crate::wayland::Portal::open(input, token.clone()).await {
                        Ok(portal) => {
                            geometry = portal.geometry.clone();
                            input = portal.input;
                            self.remote
                                .sessions
                                .lock()
                                .unwrap()
                                .update_geometry(&id, geometry.clone())?;
                            self.remote
                                .sessions
                                .lock()
                                .unwrap()
                                .limit_input(&id, input)?;
                            let (latest, _) = tokio::sync::watch::channel(None);
                            let completion = portal.start(latest.clone());
                            self.remote
                                .input_done
                                .lock()
                                .unwrap()
                                .insert(id.clone(), completion);
                            self.remote
                                .platform_frames
                                .lock()
                                .unwrap()
                                .insert(id.clone(), latest);
                            Some(portal)
                        }
                        Err(error) => {
                            write_message(
                                &mut socket,
                                &Message::RemoteDenied {
                                    reason: format!(
                                        "Wayland portal permission/capture unavailable: {error:#}"
                                    ),
                                },
                            )
                            .await?;
                            return Ok(());
                        }
                    }
                } else {
                    None
                };
                let worker = if input && native.is_none() && portal.is_none() {
                    let g = geometry.clone();
                    let t = token.clone();
                    Some(blocking(move || desktop::InputWorker::start(g, t)).await?)
                } else {
                    None
                };
                if let Some(worker) = &worker {
                    self.remote
                        .input_done
                        .lock()
                        .unwrap()
                        .insert(id.clone(), worker.completion());
                }
                if native.is_some() {
                    let (sender, _) = tokio::sync::watch::channel(None);
                    self.remote
                        .platform_frames
                        .lock()
                        .unwrap()
                        .insert(id.clone(), sender);
                }
                write_message(
                    &mut socket,
                    &Message::RemoteApproved {
                        session: id.clone(),
                        geometry,
                        input,
                        platform: std::env::consts::OS.into(),
                    },
                )
                .await?;
                self.emit(
                    "remote_active",
                    serde_json::json!({"id":id,"role":"controlled","peer":peer,"input":input}),
                );
                let result = async {
                    loop {
                        ensure!(self.trusted(&peer).await?, "trust revoked");
                        self.remote.sessions.lock().unwrap().authorize(&id, &peer)?;
                        match timeout(Duration::from_secs(35), read_message(&mut socket)).await?? {
                            Message::Ping { nonce } => {
                                write_message(&mut socket, &Message::Pong { nonce }).await?
                            }
                            Message::RemoteInput { event, geometry } => {
                                if self
                                    .remote
                                    .sessions
                                    .lock()
                                    .unwrap()
                                    .authorize(&id, &peer)?
                                    .geometry
                                    != geometry
                                {
                                    continue;
                                }
                                ensure!(
                                    self.remote.video.lock().unwrap().contains(&id),
                                    "video channel must be attached before input"
                                );
                                self.remote
                                    .sessions
                                    .lock()
                                    .unwrap()
                                    .input(&id, &peer, &event)?;
                                if let Some(worker) = &worker {
                                    worker.inject(event)?;
                                } else if let Some(portal) = &portal {
                                    portal.inject(&event).await?;
                                } else if native.is_some() {
                                    ensure!(
                                        matches!(
                                            event,
                                            RemoteMessage::Tap { .. }
                                                | RemoteMessage::Swipe { .. }
                                                | RemoteMessage::Text { .. }
                                        ),
                                        "Android supports taps, swipes and focused text only"
                                    );
                                    self.emit("platform_input",serde_json::json!({"id":id,"event":event,"geometry":geometry}));
                                } else {
                                    anyhow::bail!("view-only session");
                                }
                            }
                            Message::Bye => break,
                            _ => anyhow::bail!("invalid remote control message"),
                        }
                    }
                    Ok::<(), anyhow::Error>(())
                };
                let result = tokio::select! {_=token.cancelled()=>Ok(()),result=result=>result};
                drop(worker);
                drop(lease);
                result
            }
            Message::RemoteVideo { session } => {
                let (geometry, stop) = {
                    let sessions = self.remote.sessions.lock().unwrap();
                    let s = sessions.authorize(&session, &peer)?;
                    (s.geometry.clone(), s.stop.clone())
                };
                ensure!(
                    self.remote.video.lock().unwrap().insert(session.clone()),
                    "video channel already bound"
                );
                let _lease = VideoLease {
                    state: self.remote.clone(),
                    id: session.clone(),
                    stop: stop.clone(),
                };
                write_message(&mut socket, &Message::Ready).await?;
                let native_sender = self
                    .remote
                    .platform_frames
                    .lock()
                    .unwrap()
                    .get(&session)
                    .cloned();
                let (mut frames, producer) = if let Some(sender) = native_sender {
                    (sender.subscribe(), None)
                } else {
                    let (latest, frames) = tokio::sync::watch::channel(None::<Arc<desktop::Frame>>);
                    let producer_stop = stop.clone();
                    let producer = tokio::spawn(async move {
                        let mut timer = tokio::time::interval(Duration::from_millis(250));
                        loop {
                            tokio::select! {_=producer_stop.cancelled()=>break,_=timer.tick()=>{match blocking(desktop::capture).await{Ok(frame)=>{if frame.geometry!=geometry{producer_stop.cancel();break;}latest.send_replace(Some(Arc::new(frame)));},Err(_)=>{producer_stop.cancel();break;}}}}
                        }
                    });
                    (frames, Some(producer))
                };
                let result = async {
                    let mut sequence = 0;
                    loop {
                        frames.changed().await?;
                        let frame = frames.borrow_and_update().clone();
                        let Some(frame) = frame else { continue };
                        ensure!(self.trusted(&peer).await?, "trust revoked");
                        self.remote
                            .sessions
                            .lock()
                            .unwrap()
                            .authorize(&session, &peer)?;
                        write_message(
                            &mut socket,
                            &Message::VideoFrame {
                                session: session.clone(),
                                sequence,
                                timestamp_us: frame.timestamp_us,
                                codec: "png".into(),
                                keyframe: true,
                                width: frame.width,
                                height: frame.height,
                                geometry: frame.geometry.clone(),
                                bytes: frame.bytes.len() as u32,
                            },
                        )
                        .await?;
                        timeout(
                            Duration::from_secs(5),
                            write_video(&mut socket, sequence, &frame.bytes),
                        )
                        .await??;
                        sequence += 1;
                    }
                    #[allow(unreachable_code)]
                    Ok::<(), anyhow::Error>(())
                };
                let result = tokio::select! {_=stop.cancelled()=>Ok(()),result=result=>result};
                stop.cancel();
                if let Some(producer) = producer {
                    let _ = producer.await;
                }
                result
            }
            _ => anyhow::bail!("unsupported remote request"),
        }
    }
}
