//! Redeem an explicitly supplied one-time OpenChamber pairing link.
//!
//! The caller supplies the configured server URL; transport candidates in the
//! link are deliberately ignored. Never log the link, response, or token.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::Deserialize;
use std::time::Duration;

pub(crate) const INVALID_LINK: &str = "Invalid pairing link";
pub(crate) const REJECTED: &str = "OpenChamber rejected pairing; create a new pairing link";
pub(crate) const INVALID_RESPONSE: &str = "Invalid pairing response; create a new pairing link";
pub(crate) const UNREACHABLE: &str = "Could not reach OpenChamber; pairing may have been consumed";
pub(crate) const RATE_LIMITED: &str =
    "OpenChamber is rate limiting pairing; wait before creating a new link";
pub(crate) const UPSTREAM_FAILURE: &str =
    "OpenChamber could not redeem the pairing; retry with a new link";
pub(crate) const READ_FAILURE: &str =
    "Could not read pairing response; pairing may have been consumed, so create a new pairing link";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Pairing {
    pairing_id: String,
    secret: String,
}

pub async fn redeem(base: String, link: String) -> Result<String, &'static str> {
    tokio::task::spawn_blocking(move || redeem_blocking(&base, &link))
        .await
        .map_err(|_| "Pairing request failed")?
}

fn redeem_blocking(base: &str, link: &str) -> Result<String, &'static str> {
    if link.len() > 16384 {
        return Err(INVALID_LINK);
    }
    let uri: axum::http::Uri = link.trim().parse().map_err(|_| INVALID_LINK)?;
    if uri.scheme_str() != Some("openchamber")
        || uri.authority().map(|a| a.as_str()) != Some("connect")
    {
        return Err(INVALID_LINK);
    }
    let query = uri.query().ok_or(INVALID_LINK)?;
    let fields: Vec<_> = query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .collect();
    if fields.iter().filter(|(key, _)| *key == "v").count() != 1
        || !fields.contains(&("v", "2"))
        || fields.iter().filter(|(key, _)| *key == "p").count() != 1
    {
        return Err(INVALID_LINK);
    }
    let encoded = fields
        .iter()
        .find(|(key, _)| *key == "p")
        .ok_or(INVALID_LINK)?
        .1;
    let decoded = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| INVALID_LINK)?;
    let pairing: Pairing = serde_json::from_slice(&decoded).map_err(|_| INVALID_LINK)?;
    if pairing.pairing_id.is_empty() || pairing.secret.is_empty() {
        return Err(INVALID_LINK);
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .build()
        .into();
    let mut response = agent.post(format!("{}/api/client-auth/pairing/redeem", base.trim_end_matches('/')))
        .header("Content-Type", "application/json")
        .send(serde_json::json!({"pairingId":pairing.pairing_id,"secret":pairing.secret,"clientKind":"desktop","clientLabel":"Kanso"}).to_string())
        .map_err(|_| UNREACHABLE)?;
    if !response.status().is_success() {
        return Err(pairing_status_error(response.status().as_u16()));
    }
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|_| READ_FAILURE)?;
    let data: serde_json::Value = serde_json::from_str(&body).map_err(|_| INVALID_RESPONSE)?;
    data.get("clientToken")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
        .ok_or(INVALID_RESPONSE)
}

fn pairing_status_error(status: u16) -> &'static str {
    match status {
        400 | 404 => REJECTED,
        429 => RATE_LIMITED,
        500..=599 => UPSTREAM_FAILURE,
        _ => REJECTED,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_pairing_and_duplicate_fields_before_network() {
        for link in [
            "https://example.org/",
            "openchamber://connect?v=1&p=e30",
            "openchamber://connect?v=2&p=e30&p=e30",
        ] {
            assert!(matches!(
                redeem_blocking("http://127.0.0.1:1", link),
                Err("Invalid pairing link")
            ));
        }
    }

    #[test]
    fn pairing_statuses_have_actionable_messages() {
        assert_eq!(
            pairing_status_error(400),
            "OpenChamber rejected pairing; create a new pairing link"
        );
        assert_eq!(pairing_status_error(404), pairing_status_error(400));
        assert_eq!(
            pairing_status_error(429),
            "OpenChamber is rate limiting pairing; wait before creating a new link"
        );
        assert_eq!(
            pairing_status_error(500),
            "OpenChamber could not redeem the pairing; retry with a new link"
        );
        assert_eq!(pairing_status_error(599), pairing_status_error(500));
    }
}
