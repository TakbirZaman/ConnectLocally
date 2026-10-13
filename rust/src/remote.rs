//! Milestones 2/3 contract only. No network dispatch or native injection exists.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
    pub rotation: u16,
    pub scale: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum RemoteMessage {
    Request {
        version: u16,
        id: String,
        capabilities: Vec<String>,
    },
    Approve {
        id: String,
    },
    Deny {
        id: String,
        reason: String,
    },
    End {
        id: String,
    },
    Pointer {
        id: String,
        x: f64,
        y: f64,
    },
    Button {
        id: String,
        button: u8,
        down: bool,
    },
    Scroll {
        id: String,
        x: f64,
        y: f64,
    },
    Key {
        id: String,
        code: u32,
        down: bool,
    },
    Tap {
        id: String,
        x: f64,
        y: f64,
    },
    Swipe {
        id: String,
        x0: f64,
        y0: f64,
        x1: f64,
        y1: f64,
        duration_ms: u32,
    },
    Text {
        id: String,
        text: String,
    },
    Display {
        id: String,
        geometry: Geometry,
    },
    Frame {
        id: String,
        sequence: u64,
        timestamp_us: u64,
        codec: String,
        width: u32,
        height: u32,
        keyframe: bool,
    },
}
pub trait Capture: Send {
    fn next_frame(&mut self) -> anyhow::Result<Vec<u8>>;
    fn stop(&mut self);
}
pub trait Input: Send {
    fn inject(&mut self, event: RemoteMessage) -> anyhow::Result<()>;
    fn release_all(&mut self);
}
pub fn capabilities() -> String {
    let desktop = crate::desktop::supported();
    let portal = crate::wayland::compiled() && crate::wayland::session_is_wayland();
    serde_json::json!({"fileTransfer":true,"canViewRemote":true,"remoteInput":desktop,"screenStreaming":desktop,"waylandPortalCompiled":crate::wayland::compiled(),"captureStatus":if desktop{"available_after_local_approval"}else if portal{"requires_portal_consent_and_backend"}else if cfg!(target_os="android"){"requires_native_media_projection_consent"}else{"unsupported"},"display":if portal{"one_user_selected_monitor"}else{"primary"},"codec":"PNG","targetFps":4,"reason":if desktop{"desktop software adapter; consent required"}else if portal{"Wayland portal/PipeWire adapter compiled; backend, plugins and OS approval checked when requesting a session"}else if cfg!(target_os="android"){"Fresh MediaProjection consent required; enable Accessibility manually for supported gestures/text"}else{"capture/input adapter unavailable"}}).to_string()
}
