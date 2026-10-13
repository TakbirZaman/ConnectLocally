//! Password-authenticated pairing. The pairing protocol never transmits raw codes.
use anyhow::{ensure, Result};
use hmac::{Hmac, Mac};
use rand::Rng;
use serde::Serialize;
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};
use std::time::{Duration, Instant, SystemTime};

const LIFETIME: Duration = Duration::from_secs(120);
const REGENERATE_DELAY: Duration = Duration::from_secs(15);
const ATTEMPTS: u8 = 3;

#[derive(Serialize)]
pub struct CodeDisplay {
    pub id: String,
    pub code: String,
    pub expires_in_seconds: u64,
    pub attempts: u8,
    pub remember: bool,
}
struct Window {
    id: String,
    code: String,
    expires: Instant,
    expires_wall: SystemTime,
    attempts: u8,
    remember: bool,
}
struct Admission {
    id: String,
    expires: Instant,
    expires_wall: SystemTime,
}
#[derive(Default)]
pub struct CodeGate {
    window: Option<Window>,
    admitted: Option<Admission>,
    created: Option<Instant>,
}
pub struct Ticket {
    pub id: String,
    pub code: String,
    pub remember: bool,
}
impl CodeGate {
    pub fn begin(&mut self, remember: bool) -> Result<CodeDisplay> {
        ensure!(
            self.created
                .is_none_or(|created| created.elapsed() >= REGENERATE_DELAY),
            "Wait 15 seconds before generating another code"
        );
        let mut rng = rand::rngs::OsRng;
        let code = format!("{:06}", rng.gen_range(0..1_000_000u32));
        let id = uuid::Uuid::new_v4().to_string();
        let now = Instant::now();
        self.created = Some(now);
        self.admitted = None;
        self.window = Some(Window {
            id: id.clone(),
            code: code.clone(),
            expires: now + LIFETIME,
            expires_wall: SystemTime::now() + LIFETIME,
            attempts: ATTEMPTS,
            remember,
        });
        Ok(CodeDisplay {
            id,
            code,
            expires_in_seconds: LIFETIME.as_secs(),
            attempts: ATTEMPTS,
            remember,
        })
    }
    pub fn attempt(&mut self) -> Result<Ticket> {
        let window = self
            .window
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Generate a pairing code on the other device first"))?;
        ensure!(
            Instant::now() < window.expires
                && SystemTime::now() < window.expires_wall
                && window.attempts > 0,
            "Pairing code expired or exhausted; generate a new code"
        );
        window.attempts -= 1;
        Ok(Ticket {
            id: window.id.clone(),
            code: window.code.clone(),
            remember: window.remember,
        })
    }
    pub fn consume(&mut self, id: &str) -> Result<()> {
        ensure!(
            self.window.as_ref().is_some_and(|window| {
                window.id == id
                    && Instant::now() < window.expires
                    && SystemTime::now() < window.expires_wall
            }),
            "Pairing code expired, replaced or already used"
        );
        let window = self.window.take().unwrap();
        self.admitted = Some(Admission {
            id: window.id,
            expires: window.expires,
            expires_wall: window.expires_wall,
        });
        Ok(())
    }
    pub fn complete(&mut self, id: &str) -> Result<()> {
        ensure!(
            self.admitted.as_ref().is_some_and(|admission| {
                admission.id == id
                    && Instant::now() < admission.expires
                    && SystemTime::now() < admission.expires_wall
            }),
            "Pairing was cancelled, replaced or expired"
        );
        self.admitted = None;
        Ok(())
    }
    pub fn clear(&mut self) {
        self.window = None;
        self.admitted = None;
    }
}

pub fn validate_code(code: &str) -> Result<()> {
    ensure!(
        code.len() == 6 && code.bytes().all(|byte| byte.is_ascii_digit()),
        "Enter the six-digit code displayed on the other device"
    );
    Ok(())
}
pub fn start_a(code: &str, client: &str, server: &str) -> Result<(Spake2<Ed25519Group>, String)> {
    validate_code(code)?;
    let (state, message) = Spake2::<Ed25519Group>::start_a(
        &Password::new(code.as_bytes()),
        &Identity::new(format!("localconnect/code/v1/client/{client}").as_bytes()),
        &Identity::new(format!("localconnect/code/v1/server/{server}").as_bytes()),
    );
    Ok((state, hex::encode(message)))
}
pub fn start_b(code: &str, client: &str, server: &str) -> Result<(Spake2<Ed25519Group>, String)> {
    validate_code(code)?;
    let (state, message) = Spake2::<Ed25519Group>::start_b(
        &Password::new(code.as_bytes()),
        &Identity::new(format!("localconnect/code/v1/client/{client}").as_bytes()),
        &Identity::new(format!("localconnect/code/v1/server/{server}").as_bytes()),
    );
    Ok((state, hex::encode(message)))
}
pub fn finish(state: Spake2<Ed25519Group>, message: &str) -> Result<Vec<u8>> {
    ensure!(message.len() == 66, "invalid pairing exchange length");
    state
        .finish(&hex::decode(message)?)
        .map_err(|_| anyhow::anyhow!("invalid pairing exchange"))
}
fn confirmation(key: &[u8], channel: &[u8; 32], role: &str) -> Result<Hmac<Sha256>> {
    ensure!(role == "client" || role == "server", "invalid pairing role");
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key)
        .map_err(|_| anyhow::anyhow!("invalid pairing key"))?;
    mac.update(b"localconnect/code/v1/confirmation");
    mac.update(channel);
    mac.update(role.as_bytes());
    Ok(mac)
}
pub fn proof(key: &[u8], channel: &[u8; 32], role: &str) -> Result<String> {
    Ok(hex::encode(
        confirmation(key, channel, role)?.finalize().into_bytes(),
    ))
}
pub fn verify(key: &[u8], channel: &[u8; 32], role: &str, proof: &str) -> Result<()> {
    ensure!(proof.len() == 64, "invalid pairing proof length");
    confirmation(key, channel, role)?
        .verify_slice(&hex::decode(proof)?)
        .map_err(|_| anyhow::anyhow!("Pairing code did not match; try a new code"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn code_authenticates_channel_identity_and_role() {
        let (a, message_a) = start_a("234567", "phone", "pc").unwrap();
        let (b, message_b) = start_b("234567", "phone", "pc").unwrap();
        let key_a = finish(a, &message_b).unwrap();
        let key_b = finish(b, &message_a).unwrap();
        let channel = [7u8; 32];
        let client = proof(&key_a, &channel, "client").unwrap();
        verify(&key_b, &channel, "client", &client).unwrap();
        assert!(verify(&key_b, &[8u8; 32], "client", &client).is_err());
        assert!(verify(&key_b, &channel, "server", &client).is_err());
        let server = proof(&key_b, &channel, "server").unwrap();
        verify(&key_a, &channel, "server", &server).unwrap();
    }
    #[test]
    fn wrong_code_and_swapped_identity_fail_confirmation() {
        for (code, identity) in [("000001", "phone"), ("234567", "attacker")] {
            let (a, message_a) = start_a("234567", "phone", "pc").unwrap();
            let (b, message_b) = start_b(code, identity, "pc").unwrap();
            let key_a = finish(a, &message_b).unwrap();
            let key_b = finish(b, &message_a).unwrap();
            let client = proof(&key_a, &[0u8; 32], "client").unwrap();
            assert!(verify(&key_b, &[0u8; 32], "client", &client).is_err());
        }
    }
    #[test]
    fn bounded_attempts_expiry_single_use_and_regeneration() {
        let mut gate = CodeGate::default();
        let display = gate.begin(false).unwrap();
        validate_code(&display.code).unwrap();
        assert_eq!(display.code.len(), 6);
        assert!(gate.begin(false).is_err());
        for _ in 0..3 {
            gate.attempt().unwrap();
        }
        assert!(gate.attempt().is_err());
        gate.consume(&display.id).unwrap();
        assert!(gate.consume(&display.id).is_err());
        gate.created = Some(Instant::now() - REGENERATE_DELAY);
        gate.begin(false).unwrap();
        assert!(gate.consume(&display.id).is_err());
        gate.window.as_mut().unwrap().expires = Instant::now() - Duration::from_secs(1);
        assert!(gate.attempt().is_err());
        let id = gate.window.as_ref().unwrap().id.clone();
        assert!(gate.consume(&id).is_err());
    }
    #[test]
    fn closing_code_revokes_incomplete_admission() {
        let mut gate = CodeGate::default();
        let display = gate.begin(false).unwrap();
        gate.attempt().unwrap();
        gate.consume(&display.id).unwrap();
        gate.clear();
        assert!(gate.complete(&display.id).is_err());
        gate.created = Some(Instant::now() - REGENERATE_DELAY);
        let display = gate.begin(false).unwrap();
        gate.attempt().unwrap();
        gate.consume(&display.id).unwrap();
        gate.complete(&display.id).unwrap();
        assert!(gate.complete(&display.id).is_err());
    }
    #[test]
    fn malformed_and_replayed_messages_fail() {
        for code in ["234", "１２３４５６", "12345x"] {
            assert!(validate_code(code).is_err());
        }
        let (state, _) = start_a("123456", "a", "b").unwrap();
        assert!(finish(state, "00").is_err());
        assert!(verify(&[1; 32], &[2; 32], "client", "00").is_err());
        let mut gate = CodeGate::default();
        gate.begin(false).unwrap();
        gate.clear();
        assert!(gate.attempt().is_err());
    }
}
