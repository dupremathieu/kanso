use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::{require_human, ApiError, ApiJson, Auth};
use crate::server::AppState;

#[derive(Debug, Serialize)]
pub struct Preset {
    id: String,
    title: String,
    model: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    title: String,
    model: String,
}

fn validate(input: Input) -> Result<(String, String), ApiError> {
    let title = input.title.trim();
    if title.is_empty() || title.chars().count() > 120 || title.chars().any(char::is_control) {
        return Err(ApiError::validation(
            "title must be 1 to 120 characters without control characters",
        ));
    }
    let model = super::projects::validate_model(&input.model)?
        .ok_or_else(|| ApiError::validation("model must not be empty"))?;
    Ok((title.to_owned(), model))
}

pub(super) fn load(connection: &rusqlite::Connection) -> crate::error::Result<Vec<Preset>> {
    let mut statement = connection
        .prepare("SELECT id, title, model FROM model_presets ORDER BY title COLLATE NOCASE, id")?;
    let rows = statement
        .query_map([], |row| {
            Ok(Preset {
                id: row.get(0)?,
                title: row.get(1)?,
                model: row.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub async fn list(
    State(state): State<AppState>,
    _auth: Auth,
) -> Result<Json<Vec<Preset>>, ApiError> {
    Ok(Json(state.db.call(|connection| load(connection)).await?))
}

pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    ApiJson(input): ApiJson<Input>,
) -> Result<(StatusCode, Json<Preset>), ApiError> {
    require_human(&auth.0)?;
    let (title, model) = validate(input)?;
    let id = crate::ids::uuid_v4()?;
    let result = state
        .db
        .call(move |connection| {
            let exists: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM model_presets WHERE title = ?1 COLLATE NOCASE)",
                [&title],
                |row| row.get(0),
            )?;
            if exists {
                return Ok(None);
            }
            connection.execute(
                "INSERT INTO model_presets (id,title,model) VALUES (?1,?2,?3)",
                params![id, title, model],
            )?;
            Ok(Some(Preset { id, title, model }))
        })
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(result.ok_or_else(|| ApiError::conflict("model preset title already exists"))?),
    ))
}

pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    ApiJson(input): ApiJson<Input>,
) -> Result<Json<Preset>, ApiError> {
    require_human(&auth.0)?;
    let (title, model) = validate(input)?;
    let result = state
        .db
        .call(move |connection| {
            let found: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM model_presets WHERE id = ?1)",
                [&id],
                |row| row.get(0),
            )?;
            if !found {
                return Ok(Err(false));
            }
            let exists: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM model_presets WHERE title = ?1 COLLATE NOCASE AND id != ?2)",
                params![title, id],
                |row| row.get(0),
            )?;
            if exists {
                return Ok(Err(true));
            }
            connection.execute(
                "UPDATE model_presets SET title=?1, model=?2 WHERE id=?3",
                params![title, model, id],
            )?;
            Ok(Ok(Preset { id, title, model }))
        })
        .await?;
    Ok(Json(result.map_err(|duplicate| {
        if duplicate {
            ApiError::conflict("model preset title already exists")
        } else {
            ApiError::not_found("model preset not found")
        }
    })?))
}

pub async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_human(&auth.0)?;
    let found = state
        .db
        .call(move |connection| {
            connection
                .execute("DELETE FROM model_presets WHERE id=?1", [&id])
                .map(|n| n > 0)
                .map_err(Into::into)
        })
        .await?;
    if !found {
        return Err(ApiError::not_found("model preset not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}
