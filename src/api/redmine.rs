use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use super::{ApiError, ApiJson, ApiQuery, Auth};
use crate::{auth::IdentityKind, redmine_connections::ConnectionInfo, server::AppState};

fn browser(auth: &Auth) -> Result<(), ApiError> {
    if auth.0.kind == IdentityKind::Ui {
        Ok(())
    } else {
        Err(ApiError::forbidden("a browser session is required"))
    }
}

fn connection_error(message: &'static str) -> ApiError {
    match message {
        "connection not found" => ApiError::not_found(message),
        "cannot save Redmine settings" | "random source failed" => ApiError::internal(message),
        _ => ApiError::validation(message),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    name: String,
    url: String,
    key: Option<String>,
}

#[derive(Serialize)]
pub struct Discovery {
    account: crate::importer::redmine::Account,
    projects: Vec<crate::importer::redmine::Project>,
}

#[derive(Deserialize)]
pub struct DiscoveryQuery {
    project_id: Option<String>,
}

pub async fn account(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<crate::importer::redmine::Account>, ApiError> {
    browser(&auth)?;
    let connection = state
        .redmine
        .get(&id)
        .ok_or_else(|| ApiError::not_found("connection not found"))?;
    let revision = connection.revision;
    let account = tokio::task::spawn_blocking(move || connection.client.current_user())
        .await
        .map_err(|_| {
            ApiError::service_unavailable("provider_error", "Redmine account lookup failed")
        })?
        .map_err(|_| {
            ApiError::service_unavailable("provider_error", "Redmine account lookup failed")
        })?;
    if !state.redmine.revision_matches(&id, &revision) {
        return Err(ApiError::conflict(
            "Connection changed; retry account lookup",
        ));
    }
    Ok(Json(account))
}

pub async fn discover(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    ApiQuery(query): ApiQuery<DiscoveryQuery>,
) -> Result<Json<Discovery>, ApiError> {
    browser(&auth)?;
    let connection = state
        .redmine
        .get(&id)
        .ok_or_else(|| ApiError::validation("connection not found"))?;
    let revision = connection.revision;
    let mut discovery = tokio::task::spawn_blocking(move || {
        Ok::<_, crate::importer::ProviderError>(Discovery {
            account: connection.client.current_user()?,
            projects: connection.client.projects()?,
        })
    })
    .await
    .map_err(|_| ApiError::service_unavailable("provider_error", "Redmine discovery failed"))?
    .map_err(|_| ApiError::service_unavailable("provider_error", "Redmine discovery failed"))?;
    let _guard = state.redmine_settings_lock.lock().await;
    if !state.redmine.revision_matches(&id, &revision) {
        return Err(ApiError::conflict("Connection changed; retry discovery"));
    }
    if let Some(reference) = query.project_id {
        let connections = state.redmine.list();
        let preferred = state
            .db
            .call(move |db| {
                let Some(project) = super::projects::project_by_ref(db, &reference)? else {
                    return Ok(None);
                };
                if project.archived_at.is_some() {
                    return Ok(None);
                }
                Ok(Some(super::redmine_projects::rows(
                    db,
                    &project.id,
                    &connections,
                )?))
            })
            .await?
            .ok_or_else(|| ApiError::not_found("active project not found"))?;
        let ids: std::collections::HashSet<u64> = preferred
            .into_iter()
            .filter(|a| a.connection_id == id)
            .map(|a| a.redmine_project_id as u64)
            .collect();
        discovery.projects.sort_by_key(|p| !ids.contains(&p.id));
    }
    Ok(Json(discovery))
}

pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
) -> Result<Json<Vec<ConnectionInfo>>, ApiError> {
    browser(&auth)?;
    Ok(Json(state.redmine.list()))
}

pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    ApiJson(input): ApiJson<Input>,
) -> Result<Json<ConnectionInfo>, ApiError> {
    browser(&auth)?;
    save(state, None, input).await.map(Json)
}

pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<Input>,
) -> Result<Json<ConnectionInfo>, ApiError> {
    browser(&auth)?;
    save(state, Some(id), input).await.map(Json)
}

async fn save(
    state: AppState,
    id: Option<String>,
    input: Input,
) -> Result<ConnectionInfo, ApiError> {
    let _guard = state.redmine_settings_lock.lock().await;
    let connections = state.redmine.clone();
    let info = tokio::task::spawn_blocking(move || {
        connections.save(id.as_deref(), &input.name, &input.url, input.key.as_deref())
    })
    .await
    .map_err(|_| ApiError::internal("could not save Redmine connection"))?
    .map_err(connection_error)?;
    super::redmine_projects::prune(&state).await?;
    // Invalidate pending previews, including reservations whose upstream fetch is in flight.
    state
        .imports
        .previews
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|_, p| p.provider != crate::importer::redmine_provider::ID);
    Ok(info)
}

pub async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    browser(&auth)?;
    let _guard = state.redmine_settings_lock.lock().await;
    let connections = state.redmine.clone();
    let result = tokio::task::spawn_blocking(move || connections.delete(&id))
        .await
        .map_err(|_| ApiError::internal("could not delete Redmine connection"))?;
    if result.is_ok() || result == Err("connection not found") {
        super::redmine_projects::prune(&state).await?;
    }
    result.map_err(connection_error)?;
    state
        .imports
        .previews
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|_, p| p.provider != crate::importer::redmine_provider::ID);
    Ok(StatusCode::NO_CONTENT)
}
