use super::{load, pairing_error, update, UpdateSettings};
use crate::adapter::openchamber::OpenChamber;
use crate::api::{ApiJson, Auth};
use crate::auth::{AuthConfig, Identity, IdentityKind, Scope};
use crate::db::Database;
use crate::server::AppState;
use crate::test_support::TempDir;
use axum::extract::State;

fn state(dir: &TempDir) -> AppState {
    let db = Database::open(&dir.path().join("kanso.db")).unwrap();
    let auth = AuthConfig::load("password", None, dir.path()).unwrap();
    AppState::new(db, dir.path().join("web"), auth)
}

fn human() -> Auth {
    Auth(Identity {
        kind: IdentityKind::Ui,
        name: "ui".into(),
        scope: Scope::Human,
        display_name: None,
        session_id: None,
        token_id: None,
    })
}

#[test]
fn pairing_errors_map_to_actionable_http_statuses() {
    use crate::adapter::openchamber_pairing::{
        INVALID_LINK, INVALID_RESPONSE, RATE_LIMITED, READ_FAILURE, REJECTED, UNREACHABLE,
        UPSTREAM_FAILURE,
    };
    use axum::http::StatusCode;
    assert_eq!(
        pairing_error(INVALID_LINK).status(),
        StatusCode::BAD_REQUEST
    );
    for message in [REJECTED, INVALID_RESPONSE] {
        assert_eq!(pairing_error(message).status(), StatusCode::BAD_GATEWAY);
        assert_eq!(pairing_error(message).code(), "openchamber_error");
    }
    for message in [RATE_LIMITED, UPSTREAM_FAILURE, READ_FAILURE, UNREACHABLE] {
        assert_eq!(
            pairing_error(message).status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(pairing_error(message).code(), "pairing_failed");
    }
}

#[tokio::test]
async fn stored_connection_survives_restart_and_overrides_environment() {
    let dir = TempDir::new();
    let app = state(&dir).with_openchamber(OpenChamber::new("http://environment", "env_token"));
    load(&app).unwrap();
    assert_eq!(app.openchamber().unwrap().url(), "http://environment");
    update(
        State(app.clone()),
        human(),
        ApiJson(UpdateSettings {
            url: "http://stored".into(),
            token: Some("oc_client_stored".into()),
        }),
    )
    .await
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.path().join("openchamber-connection.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let restarted =
        state(&dir).with_openchamber(OpenChamber::new("http://other-env", "other_token"));
    load(&restarted).unwrap();
    assert_eq!(restarted.openchamber().unwrap().url(), "http://stored");
    assert_eq!(restarted.openchamber().unwrap().token(), "oc_client_stored");
}

#[test]
fn invalid_stored_token_fails_startup() {
    let dir = TempDir::new();
    let app = state(&dir).with_openchamber(OpenChamber::new("http://environment", "env_token"));
    for token in ["openchamber://connect?v=2&p=e30", "stored_token"] {
        std::fs::write(
            dir.path().join("openchamber-connection.json"),
            serde_json::json!({"url":"http://stored","token":token}).to_string(),
        )
        .unwrap();
        assert!(load(&app).unwrap_err().contains("client token"));
    }
    std::fs::write(
        dir.path().join("openchamber-connection.json"),
        r#"{"url":"file:///bad","token":"oc_client_valid"}"#,
    )
    .unwrap();
    assert!(load(&app).unwrap_err().contains("URL must be HTTP(S)"));
}
