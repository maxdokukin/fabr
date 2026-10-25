//! Cartesia speech HTTP. Headless synthesis is not the voice demo.
//! Audio paths stay inside the node and fail closed on a symlink.

use crate::tools_contract::{self, CARTESIA_KEY};
use fabr_core::FabrError;
use fabr_core::redact;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

const VERSION: &str = "2024-06-10";

pub fn speak(space: &Path, node_dir: &Path, params: &Value) -> Result<String, FabrError> {
    let transcript = params.get("transcript").and_then(|v| v.as_str()).unwrap_or("").trim();
    let voice = params.get("voice").and_then(|v| v.as_str()).unwrap_or("").trim();
    let rel = params.get("path").and_then(|v| v.as_str()).unwrap_or("").trim();
    if transcript.is_empty() || voice.is_empty() || rel.is_empty() {
        return Err(FabrError::BadRequest("cartesia_speak needs transcript, voice, and path".into()));
    }
    let key = tools_contract::secret_value(space, CARTESIA_KEY)
        .ok_or_else(|| FabrError::BadRequest("tool cartesia_speak is unavailable".into()))?;
    let body = json!({
        "model_id": "sonic-2",
        "transcript": transcript,
        "voice": {"mode": "id", "id": voice},
        "output_format": {"container": "raw", "encoding": "pcm_s16le", "sample_rate": 16000}
    });
    let bytes = post_bytes(&key, "https://api.cartesia.ai/tts/bytes", &body)?;
    if bytes.len() < 32 {
        return Err(FabrError::Remote("unavailable".into()));
    }
    let wav = pcm_wav(&bytes, 16000);
    crate::toolset::write_node_bytes(node_dir, rel, &wav).map_err(FabrError::BadRequest)?;
    Ok(format!("cartesia_speak bytes={} path={rel}", wav.len()))
}

pub fn transcribe(space: &Path, node_dir: &Path, params: &Value) -> Result<String, FabrError> {
    let rel = params.get("path").and_then(|v| v.as_str()).unwrap_or("").trim();
    if rel.is_empty() {
        return Err(FabrError::BadRequest("missing param path".into()));
    }
    let key = tools_contract::secret_value(space, CARTESIA_KEY)
        .ok_or_else(|| FabrError::BadRequest("tool cartesia_transcribe is unavailable".into()))?;
    let bytes = crate::toolset::read_node_bytes(node_dir, rel).map_err(FabrError::BadRequest)?;
    let text = stt(&key, &bytes)?;
    let text = scrub(&text, &key);
    Ok(format!("cartesia_transcribe chars={}", text.chars().count()))
}

pub fn list_voices(key: &str) -> Result<String, FabrError> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(25)).build();
    let sent = agent.get("https://api.cartesia.ai/voices").set("x-api-key", key).set("cartesia-version", VERSION).call();
    let (status, text) = read_text(sent, key)?;
    if status == 401 || status == 403 {
        return Err(FabrError::BadRequest("cartesia rejected the key".into()));
    }
    if !(200..300).contains(&status) {
        return Err(FabrError::Remote("unavailable".into()));
    }
    let value: Value = serde_json::from_str(&scrub(&text, key)).unwrap_or(Value::Null);
    let list = value.as_array().or_else(|| value.get("data").and_then(|v| v.as_array()));
    let count = list.map(|items| items.len()).unwrap_or(0);
    let voice = list.and_then(|items| items.first()).and_then(|item| item.get("id")).and_then(|id| id.as_str()).unwrap_or("");
    Ok(format!("cartesia voices={count} first_voice_present={}", !voice.is_empty()))
}

fn post_bytes(key: &str, url: &str, body: &Value) -> Result<Vec<u8>, FabrError> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(40)).build();
    let sent =
        agent.post(url).set("x-api-key", key).set("cartesia-version", VERSION).set("content-type", "application/json").send_json(body);
    match sent {
        Ok(resp) => {
            let status = resp.status();
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut resp.into_reader(), &mut bytes).map_err(|e| FabrError::Remote(e.to_string()))?;
            if (200..300).contains(&status) { Ok(bytes) } else { Err(FabrError::Remote("unavailable".into())) }
        }
        Err(ureq::Error::Status(status, _)) if status == 401 || status == 403 => {
            Err(FabrError::BadRequest("cartesia rejected the key".into()))
        }
        Err(ureq::Error::Status(408, _)) => Err(FabrError::Timeout("timeout".into())),
        Err(ureq::Error::Status(_, _)) => Err(FabrError::Remote("unavailable".into())),
        Err(err) => {
            let text = scrub(&err.to_string(), key);
            if text.to_lowercase().contains("timeout") {
                Err(FabrError::Timeout("timeout".into()))
            } else {
                Err(FabrError::Remote("unavailable".into()))
            }
        }
    }
}

fn stt(key: &str, audio: &[u8]) -> Result<String, FabrError> {
    let boundary = "fabr-cartesia";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nink-whisper\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(audio);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(40)).build();
    let sent = agent
        .post("https://api.cartesia.ai/stt")
        .set("x-api-key", key)
        .set("cartesia-version", VERSION)
        .set("content-type", &format!("multipart/form-data; boundary={boundary}"))
        .send_bytes(&body);
    let (status, text) = read_text(sent, key)?;
    if status == 401 || status == 403 {
        return Err(FabrError::BadRequest("cartesia rejected the key".into()));
    }
    if !(200..300).contains(&status) {
        return Err(FabrError::Remote("unavailable".into()));
    }
    let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Ok(value.get("text").and_then(|v| v.as_str()).unwrap_or("").trim().to_string())
}

fn read_text(sent: Result<ureq::Response, ureq::Error>, key: &str) -> Result<(u16, String), FabrError> {
    match sent {
        Ok(resp) => Ok((resp.status(), scrub(&resp.into_string().unwrap_or_default(), key))),
        Err(ureq::Error::Status(status, resp)) => Ok((status, scrub(&resp.into_string().unwrap_or_default(), key))),
        Err(err) => {
            let text = scrub(&err.to_string(), key);
            if text.to_lowercase().contains("timeout") {
                Err(FabrError::Timeout("timeout".into()))
            } else {
                Err(FabrError::Remote("unavailable".into()))
            }
        }
    }
}

fn pcm_wav(pcm: &[u8], rate: u32) -> Vec<u8> {
    let data_len = pcm.len() as u32;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    out
}

fn scrub(text: &str, key: &str) -> String {
    let out = if key.len() >= 6 { text.replace(key, "[redacted]") } else { text.to_string() };
    redact::redact(&out)
}
