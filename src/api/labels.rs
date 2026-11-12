use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::model::Label;
use super::tasks::insert_activity;
use super::{
    deserialize_present, require_human, validate_color, ApiError, ApiJson, ApiQuery, Auth,
};
use crate::ids;
use crate::server::AppState;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateLabel {
    name: String,
    color: String,
    is_global: Option<bool>,
    project_ids: Option<Vec<String>>,
    #[serde(default)]
    prompt_template: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateLabel {
    name: Option<String>,
    color: Option<String>,
    is_global: Option<bool>,
    project_ids: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_present")]
    prompt_template: Option<Option<String>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScopedLabel {
    #[serde(flatten)]
    label: Label,
    is_global: bool,
    project_ids: Vec<String>,
    prompt_template: Option<String>,
}

#[derive(Deserialize)]
pub struct LabelQuery {
    project: Option<String>,
}

fn set_scope(
    connection: &rusqlite::Connection,
    id: &str,
    global: bool,
    projects: &[String],
) -> crate::error::Result<bool> {
    for project in projects {
        if !connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1)",
            [project],
            |row| row.get::<_, bool>(0),
        )? {
            return Ok(false);
        }
    }
    connection.execute(
        "UPDATE labels SET is_global = ?1 WHERE id = ?2",
        params![global, id],
    )?;
    connection.execute("DELETE FROM label_projects WHERE label_id = ?1", [id])?;
    if !global {
        for project in projects {
            connection.execute(
                "INSERT OR IGNORE INTO label_projects (label_id, project_id) VALUES (?1, ?2)",
                params![id, project],
            )?;
        }
    }
    Ok(true)
}

fn validate_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ApiError::validation("label name must not be empty"));
    }
    if name.chars().count() > 32 {
        return Err(ApiError::validation(
            "label name must be at most 32 characters",
        ));
    }
    Ok(name.to_owned())
}

pub(super) fn label_by_id(
    connection: &rusqlite::Connection,
    id: &str,
) -> crate::error::Result<Option<ScopedLabel>> {
    let label = connection
        .query_row(
            "SELECT id, name, color, is_global, prompt_template FROM labels WHERE id = ?1",
            [id],
            |row| {
                Ok((
                    Label {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        color: row.get(2)?,
                    },
                    row.get::<_, bool>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((label, is_global, prompt_template)) = label else {
        return Ok(None);
    };
    let mut statement = connection
        .prepare("SELECT project_id FROM label_projects WHERE label_id = ?1 ORDER BY project_id")?;
    let project_ids = statement
        .query_map([id], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Some(ScopedLabel {
        label,
        is_global,
        project_ids,
        prompt_template,
    }))
}

pub async fn list(
    State(state): State<AppState>,
    _auth: Auth,
    ApiQuery(query): ApiQuery<LabelQuery>,
) -> Result<Json<Vec<ScopedLabel>>, ApiError> {
    let labels = state
        .db
        .call(move |connection| {
            if let Some(ref project) = query.project {
                let exists: bool = connection.query_row(
                    "SELECT EXISTS(SELECT 1 FROM projects WHERE id = ?1 OR key = ?1 COLLATE NOCASE)",
                    [project], |row| row.get(0),
                )?;
                if !exists { return Ok(None); }
            }
            let mut statement = connection.prepare("SELECT id FROM labels l WHERE ?1 IS NULL OR l.is_global = 1 OR EXISTS (SELECT 1 FROM label_projects lp JOIN projects p ON p.id = lp.project_id WHERE lp.label_id = l.id AND (p.id = ?1 OR p.key = ?1 COLLATE NOCASE)) ORDER BY name COLLATE NOCASE, id")?;
            let ids = statement.query_map([query.project], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let labels = ids.iter().map(|id| label_by_id(connection, id).map(|label| label.expect("listed label exists"))).collect::<crate::error::Result<Vec<_>>>()?;
            Ok(Some(labels))
        })
        .await?
        .ok_or_else(|| ApiError::not_found("project not found"))?;
    Ok(Json(labels))
}

pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    ApiJson(payload): ApiJson<CreateLabel>,
) -> Result<Response, ApiError> {
    require_human(&auth.0)?;
    let name = validate_name(&payload.name)?;
    let color = validate_color(&payload.color)?;
    let global = payload.is_global.unwrap_or(true);
    let projects = payload.project_ids.unwrap_or_default();
    let prompt_template = payload.prompt_template;
    if global && !projects.is_empty() {
        return Err(ApiError::validation(
            "global labels cannot specify projects",
        ));
    }
    let id = ids::uuid_v4()?;

    let label = state
        .db
        .call(move |connection| {
            let transaction = connection.transaction()?;
            let duplicate = transaction
                .query_row(
                    "SELECT 1 FROM labels WHERE name = ?1 COLLATE NOCASE",
                    [&name],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if duplicate {
                return Ok(None);
            }
            transaction.execute(
                "INSERT INTO labels (id, name, color, prompt_template)
                 VALUES (?1, ?2, ?3, ?4)",
                params![id, name, color, prompt_template],
            )?;
            if !set_scope(&transaction, &id, global, &projects)? {
                return Ok(Some(Err(())));
            }
            let label = label_by_id(&transaction, &id)?;
            transaction.commit()?;
            Ok(label.map(Ok))
        })
        .await?
        .ok_or_else(|| ApiError::conflict("a label with this name already exists"))?
        .map_err(|_| ApiError::validation("unknown project"))?;
    state.publish("label.created", json!({ "label": &label }));
    Ok((StatusCode::CREATED, Json(label)).into_response())
}

enum UpdateOutcome {
    Updated(ScopedLabel),
    NotFound,
    Conflict,
    UnknownProject,
    InvalidScope,
}

pub async fn update(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    ApiJson(payload): ApiJson<UpdateLabel>,
) -> Result<Json<ScopedLabel>, ApiError> {
    require_human(&auth.0)?;
    let name = payload.name.as_deref().map(validate_name).transpose()?;
    let color = payload.color.as_deref().map(validate_color).transpose()?;
    if name.is_none()
        && color.is_none()
        && payload.is_global.is_none()
        && payload.project_ids.is_none()
        && payload.prompt_template.is_none()
    {
        return Err(ApiError::validation("no editable fields provided"));
    }

    let outcome = state
        .db
        .call(move |connection| {
            let transaction = connection.transaction()?;
            let Some(label) = label_by_id(&transaction, &id)? else {
                return Ok(UpdateOutcome::NotFound);
            };
            if let Some(ref name) = name {
                let duplicate = transaction
                    .query_row(
                        "SELECT 1 FROM labels WHERE name = ?1 COLLATE NOCASE AND id <> ?2",
                        params![name, label.label.id],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if duplicate {
                    return Ok(UpdateOutcome::Conflict);
                }
                transaction.execute(
                    "UPDATE labels SET name = ?1 WHERE id = ?2",
                    params![name, label.label.id],
                )?;
            }
            if let Some(ref color) = color {
                transaction.execute(
                    "UPDATE labels SET color = ?1 WHERE id = ?2",
                    params![color, label.label.id],
                )?;
            }
            if let Some(prompt_template) = &payload.prompt_template {
                transaction.execute(
                    "UPDATE labels SET prompt_template = ?1 WHERE id = ?2",
                    params![prompt_template, label.label.id],
                )?;
            }
            if payload.is_global.is_some() || payload.project_ids.is_some() {
                let global = payload.is_global.unwrap_or(label.is_global);
                let projects = payload.project_ids.as_deref().unwrap_or(&label.project_ids);
                if global
                    && payload
                        .project_ids
                        .as_ref()
                        .is_some_and(|ids| !ids.is_empty())
                {
                    return Ok(UpdateOutcome::InvalidScope);
                }
                if !set_scope(&transaction, &label.label.id, global, projects)? {
                    return Ok(UpdateOutcome::UnknownProject);
                }
            }
            let label = label_by_id(&transaction, &label.label.id)?
                .ok_or_else(|| crate::error::Error::msg("updated label missing"))?;
            transaction.commit()?;
            Ok(UpdateOutcome::Updated(label))
        })
        .await?;

    match outcome {
        UpdateOutcome::Updated(label) => {
            state.publish("label.updated", json!({ "label": &label }));
            Ok(Json(label))
        }
        UpdateOutcome::NotFound => Err(ApiError::not_found("label not found")),
        UpdateOutcome::Conflict => Err(ApiError::conflict("a label with this name already exists")),
        UpdateOutcome::UnknownProject => Err(ApiError::validation("unknown project")),
        UpdateOutcome::InvalidScope => Err(ApiError::validation(
            "global labels cannot specify projects",
        )),
    }
}

pub async fn delete(
    State(state): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    require_human(&auth.0)?;
    let actor = auth.0.actor().to_owned();
    let deleted = state
        .db
        .call(move |connection| {
            let transaction = connection.transaction()?;
            let Some(label) = label_by_id(&transaction, &id)? else {
                return Ok(None);
            };
            let task_ids = {
                let mut statement =
                    transaction.prepare("SELECT task_id FROM task_labels WHERE label_id = ?1")?;
                let rows = statement.query_map([&label.label.id], |row| row.get::<_, String>(0))?;
                let mut task_ids = Vec::new();
                for row in rows {
                    task_ids.push(row?);
                }
                task_ids
            };
            transaction.execute("DELETE FROM labels WHERE id = ?1", [&label.label.id])?;
            for task_id in task_ids {
                insert_activity(
                    &transaction,
                    &task_id,
                    &actor,
                    "label_removed",
                    &json!({ "label_id": label.label.id, "name": label.label.name }),
                )?;
            }
            transaction.commit()?;
            Ok(Some(label))
        })
        .await?;
    if let Some(label) = deleted {
        state.publish("label.deleted", json!({ "label": &label }));
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(ApiError::not_found("label not found"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_names_are_trimmed_and_bounded() {
        assert_eq!(validate_name("  bug  ").expect("name"), "bug");
        assert!(validate_name("").is_err());
        assert!(validate_name(&"x".repeat(33)).is_err());
        assert!(validate_name(&"x".repeat(32)).is_ok());
    }
}
