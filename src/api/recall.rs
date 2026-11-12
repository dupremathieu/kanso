//! Human instructions delivered to an existing idle session, independently of
//! blocker replies. The durable comment is written before attempting delivery.
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{
    comments::{by_id, validate_body},
    model::{Comment, Task},
    now_millis, require_human,
    tasks::{column_ranks, insert_activity, load_labels, task_by_ref},
    ApiError, ApiJson, Auth,
};
use crate::{
    adapter::openchamber::{OpenChamber, PromptDelivery, SEND_TIMEOUT},
    ids, rank,
    server::AppState,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Body {
    body: String,
}

#[derive(Serialize)]
struct Delivery {
    status: &'static str,
    session_id: String,
    error: Option<String>,
    reconciled: Option<bool>,
}
#[derive(Serialize)]
pub struct Response {
    comment: Option<Comment>,
    delivery: Delivery,
}

struct Guard {
    state: AppState,
    id: String,
}
impl Drop for Guard {
    fn drop(&mut self) {
        self.state.release_dispatch(&self.id);
    }
}

struct Prepared {
    task: Task,
    revision: i64,
    comment_id: String,
    prompt: String,
    session: String,
    directory: Option<String>,
    actor: String,
}

fn eligible(task: &Task) -> Result<(), ApiError> {
    if task.state != "active" || !matches!(task.status.as_str(), "in_progress" | "in_review") {
        return Err(ApiError::conflict_code(
            "not_recallable",
            "only active in-progress or in-review cards can be recalled",
        ));
    }
    if task.session_id.is_none() || task.session_status.as_deref() != Some("idle") {
        return Err(ApiError::conflict_code(
            "session_not_idle",
            "recall requires a linked idle session",
        ));
    }
    Ok(())
}

async fn start(
    state: AppState,
    reference: String,
    actor: String,
    body: Option<String>,
    retry: Option<String>,
) -> Result<Response, ApiError> {
    let adapter = state.openchamber().ok_or_else(|| {
        ApiError::service_unavailable(
            "integration_disabled",
            "OpenChamber integration is disabled",
        )
    })?;
    let reference_lookup = reference.clone();
    let id = state
        .db
        .call(move |db| Ok(task_by_ref(db, &reference_lookup)?.map(|t| t.id)))
        .await?
        .ok_or_else(|| ApiError::not_found("task not found"))?;
    if !state.reserve_dispatch(&id) {
        return Err(ApiError::conflict_code(
            "delivery_in_progress",
            "a delivery or dispatch is already in progress",
        ));
    }
    let guard = Guard {
        state: state.clone(),
        id: id.clone(),
    };
    // The detached worker owns the reservation even if the HTTP client disconnects.
    tokio::spawn(async move {
        let _guard = guard;
        run(state, adapter, id, actor, body, retry).await
    })
    .await
    .map_err(|_| {
        ApiError::service_unavailable(
            "delivery_unknown",
            "delivery worker failed; inspect the card before retrying",
        )
    })?
}

pub async fn create(
    State(state): State<AppState>,
    auth: Auth,
    Path(reference): Path<String>,
    ApiJson(payload): ApiJson<Body>,
) -> Result<(StatusCode, Json<Response>), ApiError> {
    require_human(&auth.0)?;
    let body = validate_body(&payload.body)?;
    let result = start(
        state,
        reference,
        auth.0.actor().to_owned(),
        Some(body),
        None,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(result)))
}

pub async fn retry(
    State(state): State<AppState>,
    auth: Auth,
    Path((reference, comment_id)): Path<(String, String)>,
) -> Result<Json<Response>, ApiError> {
    require_human(&auth.0)?;
    Ok(Json(
        start(
            state,
            reference,
            auth.0.actor().to_owned(),
            None,
            Some(comment_id),
        )
        .await?,
    ))
}

async fn run(
    state: AppState,
    adapter: std::sync::Arc<OpenChamber>,
    task_id: String,
    actor: String,
    body: Option<String>,
    retry: Option<String>,
) -> Result<Response, ApiError> {
    let actor_db = actor.clone();
    let is_retry = retry.is_some();
    let prepared = state.db.call(move |db| {
        let tx = db.transaction()?;
        let Some(task) = task_by_ref(&tx, &task_id)? else { return Ok(Err(ApiError::not_found("task not found"))); };
        if let Err(error) = eligible(&task) { return Ok(Err(error)); }
        let revision = tx.query_row("SELECT recall_revision FROM tasks WHERE id=?1", [&task.id], |r| r.get(0))?;
        let session = task.session_id.clone().expect("eligible session");
        let directory = task.session_directory.clone();
        let (comment_id, prompt) = if let Some(comment_id) = retry {
            let stored: Option<(String, Option<String>, String, String)> = tx.query_row(
                "SELECT r.session_id,r.session_directory,r.prompt,c.delivery_status FROM recall_deliveries r JOIN comments c ON c.id=r.comment_id WHERE c.id=?1 AND c.task_id=?2",
                params![comment_id, task.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
            let Some((old_session, old_directory, prompt, status)) = stored else { return Ok(Err(ApiError::not_found("recall not found"))); };
            if status == "sent" { return Ok(Err(ApiError::conflict_code("already_sent", "this recall was already delivered"))); }
            if !matches!(status.as_str(), "failed" | "unknown") || old_session != session || old_directory != directory {
                return Ok(Err(ApiError::conflict_code("recall_changed", "recall session or delivery status changed")));
            }
            tx.execute("UPDATE comments SET delivery_status='unknown', delivery_error=NULL WHERE id=?1", [&comment_id])?;
            (comment_id, prompt)
        } else {
            let comment_id = ids::uuid_v4()?;
            let text = body.expect("create body");
            let prompt = format!("Human instructions on {} (recall {}):\n\n{}\n\nContinue the work. Check the card labels; if it does not have need_spec, do not ask questions in chat or card comments; if unable to proceed safely, use the blocker flow to report what is missing without asking a question.", task.reference, comment_id, text);
            tx.execute("INSERT INTO comments (id,task_id,author,body,created_at,delivery_status,delivery_kind) VALUES (?1,?2,?3,?4,?5,'unknown','recall')", params![comment_id, task.id, actor_db, text, now_millis()])?;
            tx.execute("INSERT INTO recall_deliveries (comment_id,session_id,session_directory,prompt) VALUES (?1,?2,?3,?4)", params![comment_id, session, directory, prompt])?;
            insert_activity(&tx, &task.id, &actor_db, "commented", &json!({"comment_id":comment_id}))?;
            (comment_id, prompt)
        };
        tx.commit()?;
        Ok(Ok(Prepared { task, revision, comment_id, prompt, session, directory, actor: actor_db }))
    }).await??;
    let initial = state
        .db
        .call({
            let id = prepared.comment_id.clone();
            move |db| by_id(db, &id)
        })
        .await?;
    state.publish(
        if is_retry {
            "comment.updated"
        } else {
            "comment.created"
        },
        json!({"task_id":prepared.task.id,"comment":initial}),
    );
    deliver(&state, &adapter, prepared).await
}

async fn deliver(
    state: &AppState,
    adapter: &OpenChamber,
    p: Prepared,
) -> Result<Response, ApiError> {
    let outcome = adapter
        .send_prompt(
            &p.session,
            p.directory.as_deref().unwrap_or_default(),
            &p.prompt,
            SEND_TIMEOUT,
        )
        .await;
    let (status, error) = match outcome {
        PromptDelivery::Sent => ("sent", None),
        PromptDelivery::Failed(e) => ("failed", Some(e)),
        PromptDelivery::Unknown(e) => ("unknown", Some(e)),
    };
    let session_id = p.session.clone();
    let delivery_error = error.clone();
    let task_id = p.task.id.clone();
    let old_status = p.task.status.clone();
    let (comment, updated, column) = state.db.call(move |db| {
        let tx = db.transaction()?;
        tx.execute("UPDATE comments SET delivery_status=?1, delivery_error=?2 WHERE id=?3 AND delivery_kind='recall'", params![status, error, p.comment_id])?;
        let mut updated = None;
        let mut column = None;
        if status == "sent" {
            let max: Option<f64> = tx.query_row("SELECT MAX(rank) FROM tasks WHERE project_id=?1 AND state='active' AND status='in_progress'", [&p.task.project_id], |r| r.get(0))?;
            let new_rank = if p.task.status == "in_progress" { p.task.rank } else { rank::append(max) };
            let changed = tx.execute("UPDATE tasks SET status='in_progress',rank=?1,session_status='running',attention=0,updated_at=?2 WHERE id=?3 AND state='active' AND status=?4 AND session_id=?5 AND session_directory IS ?6 AND session_status='idle' AND claimed_by IS ?7 AND recall_revision=?8 AND project_id=?9", params![new_rank, now_millis(), p.task.id, p.task.status, p.session, p.directory, p.task.claimed_by, p.revision, p.task.project_id])?;
            if changed == 1 {
                if p.task.status != "in_progress" {
                    insert_activity(&tx, &p.task.id, &p.actor, "moved", &json!({"from":p.task.status,"to":"in_progress","rank":new_rank}))?;
                    column = Some(column_ranks(&tx, &p.task.project_id, "in_progress")?);
                }
                updated = task_by_ref(&tx, &p.task.id)?;
                if let Some(task) = &mut updated {
                    task.labels = load_labels(&tx, &task.id)?;
                }
            }
        }
        insert_activity(&tx, &p.task.id, &p.actor, &format!("recall_{status}"), &json!({"comment_id":p.comment_id,"session_id":p.session,"error":error,"reconciled":if status == "sent" { Some(updated.is_some()) } else { None }}))?;
        let comment = by_id(&tx, &p.comment_id)?;
        tx.commit()?;
        Ok((comment, updated, column))
    }).await?;
    state.publish(
        "comment.updated",
        json!({"task_id":task_id,"comment":comment}),
    );
    let delivery = Delivery {
        status,
        session_id,
        error: delivery_error,
        reconciled: (status == "sent").then_some(updated.is_some()),
    };
    if let Some(task) = updated {
        if let Some(column) = column {
            state.publish("task.moved", json!({"task":task,"from":old_status,"to":"in_progress","activity":"moved","column":column}));
        } else {
            state.publish("task.updated", json!({"task":task}));
        }
    }
    Ok(Response { comment, delivery })
}
