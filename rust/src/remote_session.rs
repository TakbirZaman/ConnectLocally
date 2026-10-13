//! Consent state and validation independent of OS input/capture adapters.
//! Possession of a session ID alone is insufficient: every operation binds peer.
use crate::remote::{Geometry, RemoteMessage};
use anyhow::{ensure, Result};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

struct Pending {
    peer: String,
    geometry: Geometry,
    input: bool,
    expires: Instant,
}
pub struct Active {
    pub peer: String,
    pub geometry: Geometry,
    pub input: bool,
    pub stop: CancellationToken,
    expires: Instant,
    second: Instant,
    events: u32,
    keys: HashSet<u32>,
    buttons: HashSet<u8>,
}
#[derive(Default)]
pub struct Sessions {
    pending: HashMap<String, Pending>,
    active: HashMap<String, Active>,
}
impl Drop for Sessions {
    fn drop(&mut self) {
        for active in self.active.values() {
            active.stop.cancel();
        }
    }
}
pub struct Released {
    pub keys: Vec<u32>,
    pub buttons: Vec<u8>,
}
impl Geometry {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.width > 0 && self.height > 0 && self.width <= 16384 && self.height <= 16384,
            "invalid display bounds"
        );
        ensure!(
            matches!(self.rotation, 0 | 90 | 180 | 270)
                && self.scale.is_finite()
                && self.scale > 0.0
                && self.scale <= 8.0,
            "invalid rotation or scale"
        );
        Ok(())
    }
    pub fn map_normalized(&self, x: f64, y: f64) -> Result<(i32, i32)> {
        self.validate()?;
        ensure!(
            x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
            "coordinates outside display"
        );
        let (x, y) = match self.rotation {
            90 => (y, 1.0 - x),
            180 => (1.0 - x, 1.0 - y),
            270 => (1.0 - y, x),
            _ => (x, y),
        };
        Ok((
            (x * (self.width - 1) as f64).round() as i32,
            (y * (self.height - 1) as f64).round() as i32,
        ))
    }
}
impl Sessions {
    pub fn request(
        &mut self,
        id: String,
        peer: String,
        geometry: Geometry,
        input: bool,
    ) -> Result<()> {
        geometry.validate()?;
        uuid::Uuid::parse_str(&id)?;
        ensure!(
            peer.len() == 64 && hex::decode(&peer).is_ok(),
            "invalid authenticated peer identity"
        );
        self.pending.retain(|_, r| r.expires > Instant::now());
        ensure!(
            self.pending.len() < 4 && self.active.is_empty(),
            "another remote request/session is active"
        );
        ensure!(!self.pending.contains_key(&id), "duplicate session request");
        self.pending.insert(
            id,
            Pending {
                peer,
                geometry,
                input,
                expires: Instant::now() + Duration::from_secs(90),
            },
        );
        Ok(())
    }
    pub fn decide(&mut self, request: &str, allow: bool) -> Result<Option<String>> {
        let pending = self
            .pending
            .remove(request)
            .ok_or_else(|| anyhow::anyhow!("request expired or already decided"))?;
        if !allow || pending.expires <= Instant::now() {
            return Ok(None);
        }
        ensure!(
            self.active.is_empty(),
            "another remote session was approved"
        );
        let id = uuid::Uuid::new_v4().to_string();
        self.active.insert(
            id.clone(),
            Active {
                peer: pending.peer,
                geometry: pending.geometry,
                input: pending.input,
                stop: CancellationToken::new(),
                expires: Instant::now() + Duration::from_secs(3600),
                second: Instant::now(),
                events: 0,
                keys: HashSet::new(),
                buttons: HashSet::new(),
            },
        );
        Ok(Some(id))
    }
    pub fn authorize(&self, id: &str, peer: &str) -> Result<&Active> {
        let s = self
            .active
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("remote session not approved"))?;
        ensure!(
            s.peer == peer && s.expires > Instant::now() && !s.stop.is_cancelled(),
            "remote session peer mismatch, stopped or expired"
        );
        Ok(s)
    }
    pub fn input(&mut self, id: &str, peer: &str, event: &RemoteMessage) -> Result<()> {
        self.authorize(id, peer)?;
        let s = self.active.get_mut(id).unwrap();
        ensure!(s.input, "input capability was not approved");
        let event_id = match event {
            RemoteMessage::Pointer { id, x, y } | RemoteMessage::Tap { id, x, y } => {
                s.geometry.map_normalized(*x, *y)?;
                id
            }
            RemoteMessage::Swipe {
                id,
                x0,
                y0,
                x1,
                y1,
                duration_ms,
            } => {
                s.geometry.map_normalized(*x0, *y0)?;
                s.geometry.map_normalized(*x1, *y1)?;
                ensure!((50..=5000).contains(duration_ms), "invalid swipe duration");
                id
            }
            RemoteMessage::Button { id, button, .. } => {
                ensure!(*button < 3, "unsupported mouse button");
                id
            }
            RemoteMessage::Scroll { id, x, y } => {
                ensure!(
                    x.is_finite() && y.is_finite() && x.abs() <= 100.0 && y.abs() <= 100.0,
                    "invalid scroll"
                );
                id
            }
            RemoteMessage::Key { id, code, .. } => {
                ensure!(*code <= 0x110100, "unsupported key code");
                id
            }
            RemoteMessage::Text { id, text } => {
                ensure!(
                    text.len() <= 4096 && !text.contains('\0'),
                    "text too large or invalid"
                );
                id
            }
            _ => anyhow::bail!("not an input event"),
        };
        ensure!(event_id == id, "event belongs to another session");
        if s.second.elapsed() >= Duration::from_secs(1) {
            s.second = Instant::now();
            s.events = 0;
        }
        ensure!(s.events < 120, "remote input rate exceeded");
        s.events += 1;
        match event {
            RemoteMessage::Key { code, down, .. } => {
                if *down {
                    ensure!(
                        s.keys.len() < 256 || s.keys.contains(code),
                        "held key limit"
                    );
                    s.keys.insert(*code);
                } else {
                    s.keys.remove(code);
                }
            }
            RemoteMessage::Button { button, down, .. } => {
                if *down {
                    s.buttons.insert(*button);
                } else {
                    s.buttons.remove(button);
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn end(&mut self, id: &str) -> Released {
        if let Some(s) = self.active.remove(id) {
            s.stop.cancel();
            Released {
                keys: s.keys.into_iter().collect(),
                buttons: s.buttons.into_iter().collect(),
            }
        } else {
            Released {
                keys: vec![],
                buttons: vec![],
            }
        }
    }
    pub fn snapshot(&self) -> Vec<serde_json::Value> {
        self.active.iter().filter(|(_,s)|!s.stop.is_cancelled()&&s.expires>Instant::now()).map(|(id,s)|serde_json::json!({"id":id,"role":"controlled","peer":s.peer,"input":s.input})).collect()
    }
    pub fn revoke(&mut self, peer: &str) -> Vec<String> {
        let mut ids = self
            .pending
            .iter()
            .filter(|(_, s)| s.peer == peer)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in &ids {
            self.pending.remove(id);
        }
        let active = self
            .active
            .iter()
            .filter(|(_, s)| s.peer == peer)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for id in &active {
            self.end(id);
        }
        ids.extend(active);
        ids
    }
    pub fn deny_pending(&mut self, id: &str) {
        self.pending.remove(id);
    }
    pub fn update_pending(&mut self, id: &str, geometry: Geometry, input: bool) -> Result<()> {
        geometry.validate()?;
        let pending = self
            .pending
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("request expired"))?;
        pending.geometry = geometry;
        pending.input &= input;
        Ok(())
    }
    pub fn limit_input(&mut self, id: &str, input: bool) -> Result<()> {
        let active = self
            .active
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("session ended"))?;
        active.input &= input;
        Ok(())
    }
    pub fn update_geometry(&mut self, id: &str, geometry: Geometry) -> Result<()> {
        geometry.validate()?;
        let active = self
            .active
            .get_mut(id)
            .ok_or_else(|| anyhow::anyhow!("session ended"))?;
        ensure!(!active.stop.is_cancelled(), "session stopped");
        active.geometry = geometry;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn geometry() -> Geometry {
        Geometry {
            width: 100,
            height: 200,
            rotation: 0,
            scale: 1.0,
        }
    }
    fn approved(input: bool) -> (Sessions, String, String) {
        let mut s = Sessions::default();
        let peer = "11".repeat(32);
        let r = uuid::Uuid::new_v4().to_string();
        s.request(r.clone(), peer.clone(), geometry(), input)
            .unwrap();
        let id = s.decide(&r, true).unwrap().unwrap();
        (s, id, peer)
    }
    #[test]
    fn consent_identity_and_cleanup() {
        let (s, _, peer) = approved(true);
        assert!(s.authorize("guessed", &peer).is_err());
        let (mut s, id, peer) = approved(true);
        assert!(s.authorize(&id, &"22".repeat(32)).is_err());
        s.input(
            &id,
            &peer,
            &RemoteMessage::Key {
                id: id.clone(),
                code: 65,
                down: true,
            },
        )
        .unwrap();
        s.input(
            &id,
            &peer,
            &RemoteMessage::Button {
                id: id.clone(),
                button: 0,
                down: true,
            },
        )
        .unwrap();
        let released = s.end(&id);
        assert_eq!(released.keys, vec![65]);
        assert_eq!(released.buttons, vec![0]);
        assert!(s.authorize(&id, &peer).is_err());
    }
    #[test]
    fn denial_and_view_only() {
        let mut s = Sessions::default();
        let r = uuid::Uuid::new_v4().to_string();
        s.request(r.clone(), "11".repeat(32), geometry(), true)
            .unwrap();
        assert!(s.decide(&r, false).unwrap().is_none());
        assert!(s.decide(&r, true).is_err());
        let (mut s, id, peer) = approved(false);
        assert!(s
            .input(
                &id,
                &peer,
                &RemoteMessage::Pointer {
                    id: id.clone(),
                    x: 0.5,
                    y: 0.5
                }
            )
            .is_err());
    }
    #[test]
    fn coordinates_and_rate() {
        let (mut s, id, peer) = approved(true);
        assert!(s
            .input(
                &id,
                &peer,
                &RemoteMessage::Pointer {
                    id: id.clone(),
                    x: f64::NAN,
                    y: 0.5
                }
            )
            .is_err());
        assert!(s
            .input(
                &id,
                &peer,
                &RemoteMessage::Pointer {
                    id: "another".into(),
                    x: 0.5,
                    y: 0.5
                }
            )
            .is_err());
        for _ in 0..120 {
            s.input(
                &id,
                &peer,
                &RemoteMessage::Pointer {
                    id: id.clone(),
                    x: 0.5,
                    y: 0.5,
                },
            )
            .unwrap();
        }
        assert!(s
            .input(
                &id,
                &peer,
                &RemoteMessage::Pointer {
                    id: id.clone(),
                    x: 0.5,
                    y: 0.5
                }
            )
            .is_err());
        let mut g = geometry();
        g.rotation = 90;
        assert_eq!(g.map_normalized(0.0, 1.0).unwrap(), (99, 199));
    }
}
