//! Desktop software capture/input adapter. Primary monitor only. X11 is
//! explicitly separated from Wayland; no XWayland fallback is advertised.
use crate::remote::{Geometry, RemoteMessage};
use anyhow::{ensure, Result};
use std::sync::mpsc::{self, SyncSender};
use tokio_util::sync::CancellationToken;

pub fn supported() -> bool {
    cfg!(all(feature = "desktop-remote", target_os = "windows"))
        || (cfg!(all(feature = "desktop-remote", target_os = "linux"))
            && std::env::var_os("DISPLAY").is_some()
            && std::env::var_os("WAYLAND_DISPLAY").is_none()
            && std::env::var("XDG_SESSION_TYPE").unwrap_or_default() != "wayland")
}
#[cfg(all(
    feature = "desktop-remote",
    any(target_os = "windows", target_os = "linux")
))]
fn primary() -> Result<screenshots::Screen> {
    ensure!(
        supported(),
        "desktop capture/input unsupported in this session; Wayland requires a portal adapter"
    );
    let mut screens = screenshots::Screen::all()?;
    if screens.len() == 1 {
        return Ok(screens.remove(0));
    }
    screens
        .into_iter()
        .find(|s| s.display_info.is_primary)
        .ok_or_else(|| anyhow::anyhow!("primary display unavailable"))
}
pub fn geometry() -> Result<Geometry> {
    #[cfg(all(
        feature = "desktop-remote",
        any(target_os = "windows", target_os = "linux")
    ))]
    {
        let screen = primary()?;
        let d = screen.display_info;
        let g = Geometry {
            width: d.width,
            height: d.height,
            rotation: 0,
            scale: d.scale_factor as f64,
        };
        g.validate()?;
        ensure!(
            g.width as u64 * g.height as u64 <= 16_000_000,
            "primary display exceeds the software capture pixel limit"
        );
        Ok(g)
    }
    #[cfg(not(all(
        feature = "desktop-remote",
        any(target_os = "windows", target_os = "linux")
    )))]
    {
        anyhow::bail!("desktop adapter unavailable on this platform")
    }
}
pub struct Frame {
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub geometry: Geometry,
    pub timestamp_us: u64,
}
pub fn capture() -> Result<Frame> {
    #[cfg(all(
        feature = "desktop-remote",
        any(target_os = "windows", target_os = "linux")
    ))]
    {
        let geometry = geometry()?;
        let screen = primary()?;
        // The Windows Flutter runner is PerMonitorV2 DPI-aware: monitor bounds
        // are physical pixels. Avoid screenshots' additional scale multiplication.
        #[cfg(target_os = "windows")]
        let image = screen.capture_area_ignore_area_check(0, 0, geometry.width, geometry.height)?;
        #[cfg(target_os = "linux")]
        let image = screen.capture()?;
        ensure!(
            image.width() == geometry.width && image.height() == geometry.height,
            "display changed during capture"
        );
        let mut image = image;
        for pixel in image.pixels_mut() {
            pixel[3] = 255;
        }
        let image = screenshots::image::DynamicImage::ImageRgba8(image);
        let image = if image.width() > 1920 || image.height() > 1080 {
            image.resize(
                1920,
                1080,
                screenshots::image::imageops::FilterType::Triangle,
            )
        } else {
            image
        };
        let (width, height) = (image.width(), image.height());
        let mut encoded = std::io::Cursor::new(Vec::new());
        image.write_to(&mut encoded, screenshots::image::ImageOutputFormat::Png)?;
        let bytes = encoded.into_inner();
        ensure!(
            bytes.len() <= 8 * 1024 * 1024,
            "encoded screen frame too large"
        );
        Ok(Frame {
            bytes,
            width,
            height,
            geometry,
            timestamp_us: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_micros() as u64,
        })
    }
    #[cfg(not(all(
        feature = "desktop-remote",
        any(target_os = "windows", target_os = "linux")
    )))]
    {
        anyhow::bail!("desktop capture unavailable")
    }
}
pub struct InputWorker {
    sender: SyncSender<RemoteMessage>,
    stop: CancellationToken,
    done: tokio::sync::watch::Receiver<bool>,
}
struct Completion(tokio::sync::watch::Sender<bool>);
impl Drop for Completion {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}
impl InputWorker {
    pub fn start(geometry: Geometry, stop: CancellationToken) -> Result<Self> {
        ensure!(supported(), "desktop input unavailable");
        let (sender, receiver) = mpsc::sync_channel::<RemoteMessage>(128);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let token = stop.clone();
        let (completion, done) = tokio::sync::watch::channel(false);
        std::thread::spawn(move || {
            let _completion = Completion(completion);
            #[cfg(all(
                feature = "desktop-remote",
                any(target_os = "windows", target_os = "linux")
            ))]
            {
                use enigo::{Axis, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};
                let origin = match primary() {
                    Ok(screen) => screen.display_info,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                let mut enigo = match Enigo::new(&Settings::default()) {
                    Ok(e) => e,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.to_string()));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                let mut keys = std::collections::HashSet::new();
                let mut buttons = std::collections::HashSet::new();
                while !token.is_cancelled() {
                    let event = match receiver.recv_timeout(std::time::Duration::from_millis(50)) {
                        Ok(e) => e,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(_) => break,
                    };
                    let result = (|| -> Result<()> {
                        match event {
                            RemoteMessage::Pointer { x, y, .. } => {
                                let (x, y) = geometry.map_normalized(x, y)?;
                                enigo.move_mouse(x + origin.x, y + origin.y, Coordinate::Abs)?;
                            }
                            RemoteMessage::Tap { x, y, .. } => {
                                let (x, y) = geometry.map_normalized(x, y)?;
                                enigo.move_mouse(x + origin.x, y + origin.y, Coordinate::Abs)?;
                                enigo.button(enigo::Button::Left, Direction::Click)?;
                            }
                            RemoteMessage::Button { button, down, .. } => {
                                if down && !buttons.contains(&button) {
                                    enigo.button(button_key(button)?, Direction::Press)?;
                                    buttons.insert(button);
                                } else if !down && buttons.remove(&button) {
                                    enigo.button(button_key(button)?, Direction::Release)?;
                                }
                            }
                            RemoteMessage::Key { code, down, .. } => {
                                if down && !keys.contains(&code) {
                                    enigo.key(key(code)?, Direction::Press)?;
                                    keys.insert(code);
                                } else if !down && keys.remove(&code) {
                                    enigo.key(key(code)?, Direction::Release)?;
                                }
                            }
                            RemoteMessage::Scroll { x, y, .. } => {
                                enigo.scroll(x.round() as i32, Axis::Horizontal)?;
                                enigo.scroll(y.round() as i32, Axis::Vertical)?;
                            }
                            RemoteMessage::Text { text, .. } => enigo.text(&text)?,
                            _ => anyhow::bail!("desktop event unsupported"),
                        }
                        Ok(())
                    })();
                    if result.is_err() {
                        token.cancel();
                        break;
                    }
                }
                for code in keys {
                    if let Ok(key) = key(code) {
                        let _ = enigo.key(key, Direction::Release);
                    }
                }
                for button in buttons {
                    if let Ok(button) = button_key(button) {
                        let _ = enigo.button(button, Direction::Release);
                    }
                }
            }
            #[cfg(not(all(
                feature = "desktop-remote",
                any(target_os = "windows", target_os = "linux")
            )))]
            {
                let _ = (receiver, geometry, token);
                let _ = ready_tx.send(Err::<(), String>("unsupported".into()));
            }
        });
        ready_rx
            .recv_timeout(std::time::Duration::from_secs(3))?
            .map_err(anyhow::Error::msg)?;
        Ok(Self { sender, stop, done })
    }
    pub fn inject(&self, event: RemoteMessage) -> Result<()> {
        ensure!(!self.stop.is_cancelled(), "input worker stopped");
        self.sender
            .try_send(event)
            .map_err(|_| anyhow::anyhow!("input queue full or worker stopped"))
    }
    pub fn completion(&self) -> tokio::sync::watch::Receiver<bool> {
        self.done.clone()
    }
}
impl Drop for InputWorker {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
#[cfg(all(
    feature = "desktop-remote",
    any(target_os = "windows", target_os = "linux")
))]
fn button_key(button: u8) -> Result<enigo::Button> {
    Ok(match button {
        0 => enigo::Button::Left,
        1 => enigo::Button::Right,
        2 => enigo::Button::Middle,
        _ => anyhow::bail!("unsupported button"),
    })
}
#[cfg(all(
    feature = "desktop-remote",
    any(target_os = "windows", target_os = "linux")
))]
fn key(code: u32) -> Result<enigo::Key> {
    use enigo::Key;
    Ok(match code {
        0x110000 => Key::Return,
        0x110001 => Key::Escape,
        0x110002 => Key::Backspace,
        0x110003 => Key::Tab,
        0x110004 => Key::Control,
        0x110005 => Key::Shift,
        0x110006 => Key::Alt,
        0x110007 => Key::Meta,
        0x110008 => Key::LeftArrow,
        0x110009 => Key::RightArrow,
        0x11000a => Key::UpArrow,
        0x11000b => Key::DownArrow,
        _ => {
            let c = char::from_u32(code).ok_or_else(|| anyhow::anyhow!("invalid Unicode key"))?;
            ensure!(!c.is_control(), "control key requires named code");
            Key::Unicode(c)
        }
    })
}
