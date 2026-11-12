use axum::extract::State;
use axum::http::{header, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use super::model::{
    ChecklistItem, Comment, Dependencies, Project, Task, PROJECT_COLUMNS, TASK_COLUMNS,
};
use super::tasks::{load_checklist, load_dependencies, load_labels, load_provenance};
use super::{now_millis, require_human, ApiError, Auth};
use crate::server::AppState;
use crate::time;

/// Bump when the dump's shape changes incompatibly.
const EXPORT_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
struct ExportTask {
    #[serde(flatten)]
    task: Task,
    checklist: Vec<ChecklistItem>,
    dependencies: Dependencies,
    provenance: Option<super::model::Provenance>,
}

#[derive(Debug, Serialize)]
struct ExportComment {
    task_id: String,
    task_reference: String,
    #[serde(flatten)]
    comment: Comment,
}

#[derive(Debug, Serialize)]
struct Export {
    version: u32,
    exported_at: i64,
    projects: Vec<Project>,
    labels: Vec<super::labels::ScopedLabel>,
    tasks: Vec<ExportTask>,
    comments: Vec<ExportComment>,
    task_reference_aliases: Vec<ReferenceAlias>,
    model_presets: Vec<super::models::Preset>,
}

#[derive(Debug, Serialize)]
struct ReferenceAlias {
    project_key: String,
    number: i64,
    project_id: String,
    task_id: Option<String>,
}

/// Reuse the task detail loaders (a few small queries per task) instead of
/// introducing a second task representation; a single-user board is small.
fn load_export(connection: &mut rusqlite::Connection) -> crate::error::Result<Export> {
    let projects = {
        let mut statement = connection.prepare(&format!(
            "SELECT {PROJECT_COLUMNS} FROM projects ORDER BY position, key, id"
        ))?;
        let projects = statement
            .query_map([], Project::from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        projects
    };

    let labels = {
        let mut statement =
            connection.prepare("SELECT id FROM labels ORDER BY name COLLATE NOCASE, id")?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.iter()
            .map(|id| {
                super::labels::label_by_id(connection, id)
                    .map(|label| label.expect("listed label exists"))
            })
            .collect::<crate::error::Result<Vec<_>>>()?
    };

    let tasks = {
        let sql = format!(
            "SELECT {TASK_COLUMNS} FROM tasks t JOIN projects p ON p.id = t.project_id
             ORDER BY p.position, p.key, t.number"
        );
        let mut statement = connection.prepare(&sql)?;
        let tasks = statement
            .query_map([], Task::from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        tasks
    };

    let mut export_tasks = Vec::with_capacity(tasks.len());
    for mut task in tasks {
        task.labels = load_labels(connection, &task.id)?;
        export_tasks.push(ExportTask {
            checklist: load_checklist(connection, &task.id)?,
            dependencies: load_dependencies(connection, &task.id)?,
            provenance: load_provenance(connection, &task.id)?,
            task,
        });
    }

    let comments = {
        let mut statement = connection.prepare(
            "SELECT c.id, c.author, c.body, c.created_at, c.delivery_status, c.delivery_error,
                    c.task_id, p.key, t.number
             FROM comments c
             JOIN tasks t ON t.id = c.task_id
             JOIN projects p ON p.id = t.project_id
             ORDER BY p.position, p.key, t.number, c.created_at, c.id",
        )?;
        let comments = statement
            .query_map([], |row| {
                let key: String = row.get(7)?;
                let number: i64 = row.get(8)?;
                Ok(ExportComment {
                    task_id: row.get(6)?,
                    task_reference: format!("{key}-{number}"),
                    comment: Comment::from_row(row)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        comments
    };

    let task_reference_aliases = {
        let mut statement = connection.prepare("SELECT project_key, number, project_id, task_id FROM task_reference_aliases ORDER BY project_key, number")?;
        let aliases = statement
            .query_map([], |row| {
                Ok(ReferenceAlias {
                    project_key: row.get(0)?,
                    number: row.get(1)?,
                    project_id: row.get(2)?,
                    task_id: row.get(3)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        aliases
    };
    Ok(Export {
        version: EXPORT_VERSION,
        exported_at: now_millis(),
        projects,
        labels,
        tasks: export_tasks,
        comments,
        task_reference_aliases,
        model_presets: super::models::load(connection)?,
    })
}

/// `GET /api/export.json` — a full, versioned dump for human identities.
///
/// The whole dump is built in memory as typed structs and serialized once. A
/// single-user board is small enough that pagination or streaming would only
/// add complexity.
pub async fn download(State(state): State<AppState>, auth: Auth) -> Result<Response, ApiError> {
    require_human(&auth.0)?;
    let export = state.db.call(load_export).await?;
    let filename = format!("kanso-export-{}.json", time::file_stamp(export.exported_at));
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\""))
        .map_err(|_| ApiError::internal("could not build the export filename"))?;

    let mut response = Json(export).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(header::CONTENT_DISPOSITION, disposition);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}
