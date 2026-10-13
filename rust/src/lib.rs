#![deny(unsafe_code)]
#[cfg(all(target_os = "android", feature = "bridge"))]
mod android_jni;
#[cfg(feature = "bridge")]
pub mod api;
pub mod desktop;
pub mod engine;
pub mod errors;
#[cfg(feature = "bridge")]
#[allow(unsafe_code)] // Pinned, generated FRB FFI boundary only.
#[allow(clippy::uninlined_format_args)] // Keep pinned generator output reproducible.
mod frb_generated;
pub mod identity;
#[cfg(feature = "bridge")]
mod lifecycle;
pub mod pairing;
pub mod protocol;
pub mod remote;
pub mod remote_engine;
pub mod remote_session;
pub mod store;
pub mod transfer;
pub mod wayland;
