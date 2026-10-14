use super::db::{Database, FileQuery};
use crate::server::Server;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::json;
use std::{
    path::Path as FsPath,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use xcss::fs_safety::{PrivateDirectory, RelativePath};
use xcss::server_cli::{ContractJson, ContractPath, ContractQuery};

pub(in crate::server) fn routes() -> Router<Arc<Server>> {
    Router::new()
        .route("/status", get(status))
        .route("/scan", post(scan))
        .route("/backup", post(backup))
        .route("/files", get(files))
        .route("/file", get(file))
        .route("/folders", get(folders))
        .route("/files/{id}/tags", get(file_tags))
        .route("/files/{id}/confirm", post(confirm))
        .route("/files/{id}/relink", post(relink))
        .route("/tags", get(tags).post(create_tag))
        .route(
            "/tags/{id}",
            axum::routing::put(update_tag).delete(delete_tag),
        )
        .route("/file-tags", post(mutate_tags))
        .layer(DefaultBodyLimit::max(16 * 1024))
}

struct ApiError(StatusCode, &'static str, bool);
impl ApiError {
    fn bad() -> Self {
        Self(StatusCode::BAD_REQUEST, "invalid_request", false)
    }
    fn missing() -> Self {
        Self(StatusCode::NOT_FOUND, "not_found", false)
    }
    fn conflict() -> Self {
        Self(StatusCode::CONFLICT, "conflict", false)
    }
    fn unavailable() -> Self {
        Self(StatusCode::SERVICE_UNAVAILABLE, "root_unavailable", false)
    }
    fn internal() -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, "request_failed", false)
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (self.0, Json(json!({"code": self.1, "message": "Request could not be completed", "retryable": self.2, "details": if self.2 {json!({"retry_after": 1})} else {json!({})}}))).into_response();
        if self.2 {
            response.headers_mut().insert(
                axum::http::header::RETRY_AFTER,
                axum::http::HeaderValue::from_static("1"),
            );
        }
        response
    }
}
fn database_error(error: anyhow::Error) -> ApiError {
    if let Some(error) = error.downcast_ref::<super::db::TagMutationError>() {
        return match error {
            super::db::TagMutationError::NotFound => ApiError::missing(),
            super::db::TagMutationError::Conflict => ApiError::conflict(),
        };
    }
    if error.is::<super::db::RelinkConflict>() {
        return ApiError::conflict();
    }
    if error.is::<super::db::TagLimitExceeded>() {
        return ApiError(StatusCode::CONFLICT, "tag_limit_exceeded", false);
    }
    if error.downcast_ref::<super::capacity::Exhausted>().is_some() {
        return ApiError(StatusCode::SERVICE_UNAVAILABLE, "capacity_exhausted", false);
    }
    if error
        .downcast_ref::<super::pagination::InvalidCursor>()
        .is_some()
    {
        return ApiError::bad();
    }
    if error.downcast_ref::<super::db::Busy>().is_some() {
        return ApiError(StatusCode::TOO_MANY_REQUESTS, "too_many_requests", true);
    }
    if let Some(sqlx::Error::Database(error)) = error.downcast_ref::<sqlx::Error>()
        && error
            .code()
            .is_some_and(|code| matches!(code.as_ref(), "5" | "6" | "9"))
    {
        return ApiError(StatusCode::SERVICE_UNAVAILABLE, "service_unavailable", true);
    }
    log::warn!("Tag database operation failed");
    ApiError::internal()
}
async fn db_call<T, F>(db: Arc<Database>, task: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&Database) -> anyhow::Result<T> + Send + 'static,
{
    let permit = db
        .admission
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError(StatusCode::TOO_MANY_REQUESTS, "too_many_requests", true))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        task(&db)
    })
    .await
    .map_err(|_| ApiError::internal())?
    .map_err(database_error)
}
struct LimitedOutput(Vec<u8>);
impl std::io::Write for LimitedOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > 8 * 1024 * 1024 {
            return Err(std::io::Error::other("tag response budget exceeded"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
async fn db_json<T, F>(db: Arc<Database>, task: F) -> Result<Response, ApiError>
where
    T: serde::Serialize + Send + 'static,
    F: FnOnce(&Database) -> anyhow::Result<T> + Send + 'static,
{
    let permit = db
        .admission
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError(StatusCode::TOO_MANY_REQUESTS, "too_many_requests", true))?;
    let worker = tokio::task::spawn_blocking(move || {
        let value = task(&db)?;
        let mut output = LimitedOutput(Vec::new());
        serde_json::to_writer(&mut output, &value)?;
        Ok::<_, anyhow::Error>((output.0, permit))
    });
    let (bytes, permit) = tokio::time::timeout(std::time::Duration::from_secs(5), worker)
        .await
        .map_err(|_| ApiError(StatusCode::SERVICE_UNAVAILABLE, "service_unavailable", true))?
        .map_err(|_| ApiError::internal())?
        .map_err(database_error)?;
    // The admitted task owns the permit through cancellation; the bounded body keeps
    // it until the client consumes or drops the response.
    let stream =
        futures_util::stream::unfold((bytes, 0, permit), |(bytes, offset, permit)| async move {
            if offset == bytes.len() {
                return None;
            }
            let end = (offset + 16 * 1024).min(bytes.len());
            Some((
                Ok::<_, std::convert::Infallible>(axum::body::Bytes::copy_from_slice(
                    &bytes[offset..end],
                )),
                (bytes, end, permit),
            ))
        });
    let mut response = axum::body::Body::from_stream(stream).into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    Ok(response)
}

async fn status(State(server): State<Arc<Server>>) -> Result<Response, ApiError> {
    let live = server.tagging.live_root().is_ok();
    db_json(server.tagging.db.clone(), move |db| {
        let mut value = db.status()?;
        if !live {
            value.available = false;
        }
        Ok(value)
    })
    .await
}
async fn scan(State(server): State<Arc<Server>>) -> Result<Json<serde_json::Value>, ApiError> {
    let count = server.tagging.scan_once().await.map_err(|_error| {
        log::warn!("The manual tag scan failed");
        ApiError::unavailable()
    })?;
    Ok(Json(json!({"scanned": count})))
}
async fn backup(State(server): State<Arc<Server>>) -> Result<Json<serde_json::Value>, ApiError> {
    let state = server.tagging.state_dir.clone();
    let db = server.tagging.db.clone();
    db_call(db, move |db| {
        let backup_dir = state.join("tag-backups");
        let _private = PrivateDirectory::create(&backup_dir)?;
        let time = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let filename = format!("tags-{time}-{}.db", uuid::Uuid::new_v4());
        db.backup(&backup_dir.join(&filename))?;
        Ok(filename)
    })
    .await
    .map(|filename| Json(json!({"filename": filename})))
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FilesQuery {
    #[serde(default)]
    search: String,
    #[serde(default)]
    directory: String,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    all: String,
    #[serde(default)]
    any: String,
    #[serde(default)]
    exclude: String,
    page: Option<u32>,
    page_size: Option<u32>,
}
fn parse_ids(value: &str) -> Result<Vec<i64>, ApiError> {
    if value.is_empty() {
        return Ok(vec![]);
    }
    let ids = value
        .split(',')
        .map(|item| item.parse::<i64>().map_err(|_| ApiError::bad()))
        .collect::<Result<Vec<_>, _>>()?;
    if ids.len() > 100 || ids.iter().any(|id| *id < 1) {
        return Err(ApiError::bad());
    }
    Ok(ids)
}
fn valid_directory(value: &str) -> bool {
    value.is_empty() || RelativePath::new(FsPath::new(value)).is_ok()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePathQuery {
    path: String,
}
async fn file(
    State(server): State<Arc<Server>>,
    ContractQuery(q): ContractQuery<FilePathQuery>,
) -> Result<Json<super::db::ListedTags>, ApiError> {
    if q.path.is_empty() || q.path.len() > 4096 || !valid_directory(&q.path) {
        return Err(ApiError::bad());
    }
    server
        .tags_for_file(&q.path)
        .await
        .map(Json)
        .map_err(database_error)
}
async fn files(
    State(server): State<Arc<Server>>,
    ContractQuery(q): ContractQuery<FilesQuery>,
) -> Result<Response, ApiError> {
    if !valid_directory(&q.directory) || q.search.len() > 200 {
        return Err(ApiError::bad());
    }
    let query = FileQuery {
        search: q.search,
        directory: q.directory,
        scope: if q.scope.is_empty() {
            "current".into()
        } else {
            q.scope
        },
        status: if q.status.is_empty() {
            "present".into()
        } else {
            q.status
        },
        all_tags: parse_ids(&q.all)?,
        any_tags: parse_ids(&q.any)?,
        exclude_tags: parse_ids(&q.exclude)?,
        page: q.page.unwrap_or(1),
        page_size: q.page_size.unwrap_or(50),
    };
    if query.page == 0
        || query.page_size == 0
        || query.page_size > 100
        || !matches!(query.scope.as_str(), "current" | "recursive" | "all")
        || !matches!(
            query.status.as_str(),
            "present" | "missing" | "suspect" | "all"
        )
    {
        return Err(ApiError::bad());
    }
    db_json(server.tagging.db.clone(), move |db| db.list(&query)).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FolderQuery {
    #[serde(default)]
    path: String,
    cursor: Option<String>,
}
async fn folders(
    State(server): State<Arc<Server>>,
    ContractQuery(q): ContractQuery<FolderQuery>,
) -> Result<Response, ApiError> {
    if !valid_directory(&q.path) {
        return Err(ApiError::bad());
    }
    db_json(server.tagging.db.clone(), move |db| {
        db.folders(&q.path, q.cursor.as_deref())
    })
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    cursor: Option<String>,
}
async fn tags(
    State(server): State<Arc<Server>>,
    ContractQuery(q): ContractQuery<PageQuery>,
) -> Result<Response, ApiError> {
    db_json(server.tagging.db.clone(), move |db| {
        db.tags(q.cursor.as_deref())
    })
    .await
}
async fn file_tags(
    State(server): State<Arc<Server>>,
    ContractPath(id): ContractPath<i64>,
    ContractQuery(q): ContractQuery<PageQuery>,
) -> Result<Response, ApiError> {
    if id < 1 {
        return Err(ApiError::bad());
    }
    db_json(server.tagging.db.clone(), move |db| {
        db.file_tags_page(id, q.cursor.as_deref())
    })
    .await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TagInput {
    name: String,
    color: Option<String>,
}
fn validate_tag(input: &TagInput) -> Result<(), ApiError> {
    let name = input.name.trim();
    if name != input.name
        || name.is_empty()
        || name.chars().count() > 80
        || name.chars().any(char::is_control)
    {
        return Err(ApiError::bad());
    }
    if let Some(color) = &input.color
        && (color.len() != 7
            || !color.starts_with('#')
            || !color.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit))
    {
        return Err(ApiError::bad());
    }
    Ok(())
}
async fn create_tag(
    State(server): State<Arc<Server>>,
    ContractJson(input): ContractJson<TagInput>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    validate_tag(&input)?;
    let id = db_call(server.tagging.db.clone(), move |db| {
        db.create_tag(&input.name, input.color.as_deref())
    })
    .await?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}
async fn update_tag(
    State(server): State<Arc<Server>>,
    ContractPath(id): ContractPath<i64>,
    ContractJson(input): ContractJson<TagInput>,
) -> Result<StatusCode, ApiError> {
    validate_tag(&input)?;
    if db_call(server.tagging.db.clone(), move |db| {
        db.update_tag(id, &input.name, input.color.as_deref())
    })
    .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::missing())
    }
}
async fn delete_tag(
    State(server): State<Arc<Server>>,
    ContractPath(id): ContractPath<i64>,
) -> Result<StatusCode, ApiError> {
    if db_call(server.tagging.db.clone(), move |db| db.delete_tag(id)).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::missing())
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TagMutation {
    file_ids: Vec<i64>,
    tag_ids: Vec<i64>,
    action: String,
}
async fn mutate_tags(
    State(server): State<Arc<Server>>,
    ContractJson(input): ContractJson<TagMutation>,
) -> Result<StatusCode, ApiError> {
    if input.file_ids.is_empty()
        || input.file_ids.len() > 500
        || input.tag_ids.is_empty()
        || input.tag_ids.len() > 100
        || input
            .file_ids
            .iter()
            .chain(input.tag_ids.iter())
            .any(|id| *id < 1)
        || !matches!(input.action.as_str(), "add" | "remove")
    {
        return Err(ApiError::bad());
    }
    let add = input.action == "add";
    db_call(server.tagging.db.clone(), move |db| {
        db.mutate_tags(&input.file_ids, &input.tag_ids, add)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelinkInput {
    source_id: i64,
}
async fn relink(
    State(server): State<Arc<Server>>,
    ContractPath(id): ContractPath<i64>,
    ContractJson(input): ContractJson<RelinkInput>,
) -> Result<StatusCode, ApiError> {
    db_call(server.tagging.db.clone(), move |db| {
        db.relink(input.source_id, id)
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn confirm(
    State(server): State<Arc<Server>>,
    ContractPath(id): ContractPath<i64>,
) -> Result<StatusCode, ApiError> {
    if db_call(server.tagging.db.clone(), move |db| db.confirm(id)).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::conflict())
    }
}

#[cfg(test)]
mod tests;
