use std::collections::HashSet;

use axum::{
    extract::{Path, State},
    Json,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::{projects::project_by_ref, require_human, ApiError, ApiJson, Auth};
use crate::{redmine_connections::ConnectionInfo, server::AppState};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Association {
    pub connection_id: String,
    pub redmine_project_id: i64,
    /// Optional optimistic guard for clients editing a discovery snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_url: Option<String>,
}

pub(super) fn rows(
    db: &rusqlite::Connection,
    project: &str,
    connections: &[ConnectionInfo],
) -> crate::error::Result<Vec<Association>> {
    let mut stmt = db.prepare("SELECT connection_id, connection_url, redmine_project_id FROM redmine_projects WHERE project_id=?1 ORDER BY connection_id, redmine_project_id")?;
    let stored = stmt
        .query_map([project], |row| {
            Ok((
                Association {
                    connection_id: row.get(0)?,
                    redmine_project_id: row.get(2)?,
                    connection_url: None,
                },
                row.get::<_, String>(1)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    // URL binding prevents old IDs applying to a different server even if a
    // process stops between updating the connection file and database cleanup.
    Ok(stored
        .into_iter()
        .filter(|(a, url)| {
            connections
                .iter()
                .any(|c| c.id == a.connection_id && c.url == *url)
        })
        .map(|(a, _)| a)
        .collect())
}

/// Reconcile file-owned connections with database-owned preferences. Running
/// this on every connection save/delete makes cleanup retryable after a crash
/// or database failure, including when a repeated delete finds no connection.
/// The caller must hold `redmine_settings_lock`.
pub(super) async fn prune(state: &AppState) -> Result<(), ApiError> {
    let connections = state.redmine.list();
    state
        .db
        .call(move |db| {
            let tx = db.transaction()?;
            let stored = {
                let mut stmt = tx.prepare(
                    "SELECT DISTINCT connection_id, connection_url FROM redmine_projects",
                )?;
                let pairs = stmt
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                pairs
            };
            for (id, url) in stored {
                if !connections.iter().any(|c| c.id == id && c.url == url) {
                    tx.execute(
                        "DELETE FROM redmine_projects WHERE connection_id=?1 AND connection_url=?2",
                        params![id, url],
                    )?;
                }
            }
            tx.commit()?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub async fn list(
    State(state): State<AppState>,
    auth: Auth,
    Path(reference): Path<String>,
) -> Result<Json<Vec<Association>>, ApiError> {
    require_human(&auth.0)?;
    let _guard = state.redmine_settings_lock.lock().await;
    let connections = state.redmine.list();
    let result = state
        .db
        .call(move |db| {
            let Some(project) = project_by_ref(db, &reference)? else {
                return Ok(None);
            };
            Ok(Some(rows(db, &project.id, &connections)?))
        })
        .await?;
    Ok(Json(
        result.ok_or_else(|| ApiError::not_found("project not found"))?,
    ))
}

pub async fn replace(
    State(state): State<AppState>,
    auth: Auth,
    Path(reference): Path<String>,
    ApiJson(associations): ApiJson<Vec<Association>>,
) -> Result<Json<Vec<Association>>, ApiError> {
    require_human(&auth.0)?;
    let _guard = state.redmine_settings_lock.lock().await;
    let connections = state.redmine.list();
    let mut seen = HashSet::new();
    if associations.len() > 1000
        || associations.iter().any(|a| {
            a.redmine_project_id <= 0
                || !connections.iter().any(|c| c.id == a.connection_id)
                || !seen.insert((a.connection_id.clone(), a.redmine_project_id))
        })
    {
        return Err(ApiError::validation(
            "Invalid or duplicate Redmine project association (maximum 1000)",
        ));
    }
    if associations.iter().any(|a| {
        a.connection_url.as_ref().is_some_and(|url| {
            !connections
                .iter()
                .any(|c| c.id == a.connection_id && c.url == *url)
        })
    }) {
        return Err(ApiError::conflict(
            "Connection changed; reload project settings",
        ));
    }
    let result = state.db.call(move |db| {
        let tx = db.transaction()?;
        let Some(project) = project_by_ref(&tx, &reference)? else { return Ok(None) };
        if project.archived_at.is_some() { return Ok(None); }
        tx.execute("DELETE FROM redmine_projects WHERE project_id=?1", [&project.id])?;
        for a in associations {
            let connection = connections.iter().find(|c| c.id == a.connection_id).expect("validated connection");
            tx.execute("INSERT INTO redmine_projects (project_id, connection_id, connection_url, redmine_project_id) VALUES (?1,?2,?3,?4)",
                params![project.id, a.connection_id, connection.url, a.redmine_project_id])?;
        }
        let result = rows(&tx, &project.id, &connections)?;
        tx.commit()?;
        Ok(Some(result))
    }).await?;
    Ok(Json(result.ok_or_else(|| {
        ApiError::not_found("active project not found")
    })?))
}
