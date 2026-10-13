//! Explicit Wayland portal adapter; no XWayland/XTest fallback.
use crate::{
    desktop::Frame,
    remote::{Geometry, RemoteMessage},
};
use anyhow::Result;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;
pub fn session_is_wayland() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").unwrap_or_default() == "wayland")
}
pub fn compiled() -> bool {
    cfg!(all(target_os = "linux", feature = "wayland-remote"))
}
#[cfg(all(target_os = "linux", feature = "wayland-remote"))]
mod adapter {
    use super::*;
    use anyhow::{ensure, Context};
    use ashpd::desktop::{
        remote_desktop::{DeviceType, KeyState, RemoteDesktop},
        screencast::{CursorMode, Screencast, SourceType},
        PersistMode, Session,
    };
    use gstreamer::{self as gst, prelude::*};
    use std::{
        collections::HashSet,
        os::fd::{AsRawFd, OwnedFd},
        sync::Mutex,
    };
    struct SessionGuard(Option<Arc<Session<'static, RemoteDesktop<'static>>>>);
    impl Drop for SessionGuard {
        fn drop(&mut self) {
            if let (Some(session), Ok(runtime)) =
                (self.0.take(), tokio::runtime::Handle::try_current())
            {
                runtime.spawn(async move {
                    let _ = session.close().await;
                });
            }
        }
    }
    pub struct Portal {
        proxy: RemoteDesktop<'static>,
        session: Arc<Session<'static, RemoteDesktop<'static>>>,
        pipeline: gst::Pipeline,
        sink: gstreamer_app::AppSink,
        _fd: OwnedFd,
        node: u32,
        pub geometry: Geometry,
        pub input: bool,
        width: u32,
        height: u32,
        stop: CancellationToken,
        serial: tokio::sync::Mutex<()>,
        keys: Mutex<HashSet<u32>>,
        buttons: Mutex<HashSet<u8>>,
    }
    impl Portal {
        pub async fn open(input: bool, stop: CancellationToken) -> Result<Arc<Self>> {
            gst::init()?;
            let proxy = RemoteDesktop::new()
                .await
                .context("Install a RemoteDesktop-capable xdg-desktop-portal backend")?;
            let cast = Screencast::new().await?;
            let session = Arc::new(proxy.create_session().await?);
            let mut guard = SessionGuard(Some(session.clone()));
            let setup = async {
                let types = if input {
                    DeviceType::Keyboard | DeviceType::Pointer
                } else {
                    DeviceType::Pointer.into()
                };
                // A view-only session never accepts input, even if the portal grants pointer access.
                proxy
                    .select_devices(&session, types, None, PersistMode::DoNot)
                    .await?
                    .response()?;
                cast.select_sources(
                    &session,
                    CursorMode::Embedded,
                    SourceType::Monitor.into(),
                    false,
                    None,
                    PersistMode::DoNot,
                )
                .await?
                .response()?;
                let selected = proxy.start(&session, None).await?.response()?;
                let streams = selected
                    .streams()
                    .context("Portal returned no screen stream")?;
                ensure!(streams.len() == 1, "Select exactly one monitor");
                let stream = &streams[0];
                let (w, h) = stream
                    .size()
                    .context("Portal did not report monitor coordinate bounds")?;
                ensure!(
                    w > 0 && h > 0 && w <= 16384 && h <= 16384,
                    "Unsupported portal monitor bounds"
                );
                let geometry = Geometry {
                    width: w as u32,
                    height: h as u32,
                    rotation: 0,
                    scale: 1.0,
                };
                let input = input
                    && selected.devices().contains(DeviceType::Keyboard)
                    && selected.devices().contains(DeviceType::Pointer);
                let node = stream.pipe_wire_node_id();
                let fd = cast.open_pipe_wire_remote(&session).await?;
                let ratio = 1.0f64.min(1920.0 / w as f64).min(1080.0 / h as f64);
                let width = ((w as f64 * ratio) as u32).max(1);
                let height = ((h as f64 * ratio) as u32).max(1);
                let source = gst::ElementFactory::make("pipewiresrc")
                    .property("fd", fd.as_raw_fd())
                    .property("path", node.to_string())
                    .property("do-timestamp", true)
                    .build()
                    .context("Install gstreamer1.0-pipewire")?;
                let queue = gst::ElementFactory::make("queue")
                    .property("max-size-buffers", 1u32)
                    .property_from_str("leaky", "downstream")
                    .build()?;
                let convert = gst::ElementFactory::make("videoconvert").build()?;
                let scale = gst::ElementFactory::make("videoscale").build()?;
                let rate = gst::ElementFactory::make("videorate")
                    .property("drop-only", true)
                    .build()?;
                let filter = gst::ElementFactory::make("capsfilter")
                    .property(
                        "caps",
                        gst::Caps::builder("video/x-raw")
                            .field("width", width as i32)
                            .field("height", height as i32)
                            .field("framerate", gst::Fraction::new(4, 1))
                            .build(),
                    )
                    .build()?;
                let encoder = gst::ElementFactory::make("pngenc")
                    .build()
                    .context("Install GStreamer good plugins (pngenc)")?;
                let sink = gstreamer_app::AppSink::builder()
                    .max_buffers(1)
                    .drop(true)
                    .sync(false)
                    .build();
                let pipeline = gst::Pipeline::new();
                let elements = [
                    &source,
                    &queue,
                    &convert,
                    &scale,
                    &rate,
                    &filter,
                    &encoder,
                    sink.upcast_ref(),
                ];
                pipeline.add_many(elements)?;
                gst::Element::link_many(elements)?;
                pipeline.set_state(gst::State::Playing)?;
                Ok::<_, anyhow::Error>((geometry, input, node, fd, width, height, pipeline, sink))
            };
            let result = tokio::select! {_=stop.cancelled()=>Err(anyhow::anyhow!("cancelled")),r=tokio::time::timeout(std::time::Duration::from_secs(75),setup)=>r.map_err(anyhow::Error::from).and_then(|r|r)};
            let (geometry, input, node, fd, width, height, pipeline, sink) = match result {
                Ok(result) => result,
                Err(error) => {
                    let _ = session.close().await;
                    return Err(error);
                }
            };
            guard.0 = None;
            Ok(Arc::new(Self {
                proxy,
                session,
                pipeline,
                sink,
                _fd: fd,
                node,
                geometry,
                input,
                width,
                height,
                stop,
                serial: tokio::sync::Mutex::new(()),
                keys: Mutex::new(HashSet::new()),
                buttons: Mutex::new(HashSet::new()),
            }))
        }
        pub fn start(
            self: &Arc<Self>,
            latest: tokio::sync::watch::Sender<Option<Arc<Frame>>>,
        ) -> tokio::sync::watch::Receiver<bool> {
            let (done, completion) = tokio::sync::watch::channel(false);
            let capture = self.clone();
            let capture_thread = std::thread::spawn(move || {
                while !capture.stop.is_cancelled() {
                    if let Some(message) = capture.pipeline.bus().and_then(|bus| {
                        bus.timed_pop_filtered(
                            gst::ClockTime::ZERO,
                            &[gst::MessageType::Error, gst::MessageType::Eos],
                        )
                    }) {
                        let _ = message;
                        capture.stop.cancel();
                        break;
                    }
                    let Some(sample) = capture
                        .sink
                        .try_pull_sample(gst::ClockTime::from_mseconds(100))
                    else {
                        continue;
                    };
                    let result = (|| -> Result<Frame> {
                        let map = sample
                            .buffer()
                            .context("Missing portal frame")?
                            .map_readable()?;
                        ensure!(
                            map.len() <= 8 * 1024 * 1024 && map.len() >= 24,
                            "Portal frame size exceeds limit"
                        );
                        let bytes = map.as_slice();
                        ensure!(
                            bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
                            "Portal encoder did not produce PNG"
                        );
                        ensure!(
                            u32::from_be_bytes(bytes[16..20].try_into()?) == capture.width
                                && u32::from_be_bytes(bytes[20..24].try_into()?) == capture.height,
                            "Portal stream changed; request a new session"
                        );
                        Ok(Frame {
                            bytes: bytes.to_vec(),
                            width: capture.width,
                            height: capture.height,
                            geometry: capture.geometry.clone(),
                            timestamp_us: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)?
                                .as_micros() as u64,
                        })
                    })();
                    match result {
                        Ok(frame) => {
                            latest.send_replace(Some(Arc::new(frame)));
                        }
                        Err(_) => {
                            capture.stop.cancel();
                            break;
                        }
                    }
                }
            });
            let cleanup = self.clone();
            tokio::spawn(async move {
                cleanup.stop.cancelled().await;
                let _serial = cleanup.serial.lock().await;
                let keys = cleanup.keys.lock().unwrap().drain().collect::<Vec<_>>();
                let buttons = cleanup.buttons.lock().unwrap().drain().collect::<Vec<_>>();
                let release = async {
                    for code in keys {
                        if let Ok(code) = keysym(code) {
                            let _ = cleanup
                                .proxy
                                .notify_keyboard_keysym(&cleanup.session, code, KeyState::Released)
                                .await;
                        }
                    }
                    for button in buttons {
                        let _ = cleanup
                            .proxy
                            .notify_pointer_button(
                                &cleanup.session,
                                button_code(button),
                                KeyState::Released,
                            )
                            .await;
                    }
                    let _ = cleanup.session.close().await;
                };
                let _ = tokio::time::timeout(std::time::Duration::from_secs(2), release).await;
                let pipeline = cleanup.pipeline.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    let state = pipeline.set_state(gst::State::Null);
                    let _ = capture_thread.join();
                    state
                })
                .await;
                done.send_replace(true);
            });
            completion
        }
        pub async fn inject(&self, event: &RemoteMessage) -> Result<()> {
            let _serial = self.serial.lock().await;
            ensure!(
                self.input && !self.stop.is_cancelled(),
                "Portal input was not approved or is stopped"
            );
            match event {
                RemoteMessage::Pointer { x, y, .. } => {
                    let (x, y) = self.geometry.map_normalized(*x, *y)?;
                    self.proxy
                        .notify_pointer_motion_absolute(
                            &self.session,
                            self.node,
                            x as f64,
                            y as f64,
                        )
                        .await?;
                }
                RemoteMessage::Button { button, down, .. } => {
                    let changed = if *down {
                        self.buttons.lock().unwrap().insert(*button)
                    } else {
                        self.buttons.lock().unwrap().remove(button)
                    };
                    if changed {
                        self.proxy
                            .notify_pointer_button(
                                &self.session,
                                button_code(*button),
                                if *down {
                                    KeyState::Pressed
                                } else {
                                    KeyState::Released
                                },
                            )
                            .await?;
                    }
                }
                RemoteMessage::Key { code, down, .. } => {
                    let symbol = keysym(*code)?;
                    let changed = if *down {
                        self.keys.lock().unwrap().insert(*code)
                    } else {
                        self.keys.lock().unwrap().remove(code)
                    };
                    if changed {
                        self.proxy
                            .notify_keyboard_keysym(
                                &self.session,
                                symbol,
                                if *down {
                                    KeyState::Pressed
                                } else {
                                    KeyState::Released
                                },
                            )
                            .await?;
                    }
                }
                RemoteMessage::Scroll { x, y, .. } => {
                    self.proxy
                        .notify_pointer_axis(&self.session, *x, *y, true)
                        .await?
                }
                RemoteMessage::Text { text, .. } => {
                    ensure!(
                        text.chars().count() <= 256,
                        "Wayland text limited to 256 characters per action"
                    );
                    for c in text.chars() {
                        ensure!(!self.stop.is_cancelled(), "Portal stopped");
                        ensure!(
                            !c.is_control(),
                            "Portal text supports printable Unicode only"
                        );
                        let code = keysym(c as u32)?;
                        self.keys.lock().unwrap().insert(c as u32);
                        self.proxy
                            .notify_keyboard_keysym(&self.session, code, KeyState::Pressed)
                            .await?;
                        self.proxy
                            .notify_keyboard_keysym(&self.session, code, KeyState::Released)
                            .await?;
                        self.keys.lock().unwrap().remove(&(c as u32));
                    }
                }
                _ => anyhow::bail!("Wayland input event unsupported"),
            }
            Ok(())
        }
    }
    impl Drop for Portal {
        fn drop(&mut self) {
            self.stop.cancel();
            let _ = self.pipeline.set_state(gst::State::Null);
        }
    }
    fn button_code(button: u8) -> i32 {
        match button {
            0 => 272,
            1 => 273,
            _ => 274,
        }
    }
    fn keysym(code: u32) -> Result<i32> {
        Ok(match code {
            0x110000 => 0xff0d,
            0x110001 => 0xff1b,
            0x110002 => 0xff08,
            0x110003 => 0xff09,
            0x110004 => 0xffe3,
            0x110005 => 0xffe1,
            0x110006 => 0xffe9,
            0x110007 => 0xffeb,
            0x110008 => 0xff51,
            0x110009 => 0xff53,
            0x11000a => 0xff52,
            0x11000b => 0xff54,
            _ => {
                let c = char::from_u32(code).context("Unsupported portal key")?;
                ensure!(!c.is_control(), "Named code required for control key");
                if code <= 255 {
                    code as i32
                } else {
                    (0x01000000 | code) as i32
                }
            }
        })
    }
}
#[cfg(all(target_os = "linux", feature = "wayland-remote"))]
pub use adapter::Portal;
#[cfg(not(all(target_os = "linux", feature = "wayland-remote")))]
pub struct Portal {
    pub geometry: Geometry,
    pub input: bool,
}
#[cfg(not(all(target_os = "linux", feature = "wayland-remote")))]
impl Portal {
    pub async fn open(_: bool, _: CancellationToken) -> Result<Arc<Self>> {
        anyhow::bail!("Wayland portal adapter not compiled")
    }
    pub fn start(
        &self,
        _: tokio::sync::watch::Sender<Option<Arc<Frame>>>,
    ) -> tokio::sync::watch::Receiver<bool> {
        tokio::sync::watch::channel(true).1
    }
    pub async fn inject(&self, _: &RemoteMessage) -> Result<()> {
        anyhow::bail!("Wayland portal adapter not compiled")
    }
}
