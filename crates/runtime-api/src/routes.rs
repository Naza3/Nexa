use crate::{
    ApiState,
    dto::{ImportModelRequest, LoadRequest},
    errors::ApiError,
    security::SecurityContext,
};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{
        Path, Query, Request, State,
        rejection::{PathRejection, QueryRejection},
    },
    http::StatusCode,
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use runtime_types::{ModelId, ModelState, RequestId, RuntimeStatus};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn router(state: ApiState, security: Arc<SecurityContext>) -> Router {
    let body_limit = crate::security::BodyLimit(state.config.api.max_body_bytes);
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/models", get(available_models))
        .route("/v1/chat/completions", post(crate::chat::chat))
        .route("/runtime/status", get(status))
        .route(
            "/runtime/configuration",
            get(configuration_get).put(configuration_save),
        )
        .route(
            "/runtime/configuration/models/{model_id}",
            get(configuration_model_get),
        )
        .route("/runtime/devices", get(devices))
        .route("/runtime/models", get(models))
        .route("/runtime/models/import", post(import))
        .route("/runtime/models/unregister", post(unregister))
        .route("/runtime/load", post(load))
        .route("/runtime/load-operations", post(load_operation_start))
        .route("/runtime/load-operations/{id}", get(load_operation_next))
        .route(
            "/runtime/load-operations/{id}/cancel",
            post(load_operation_cancel),
        )
        .route("/runtime/load-and-test", post(load_and_test))
        .route("/runtime/load-if-unloaded", post(load_if_unloaded))
        .route("/runtime/model-test", post(model_test))
        .route("/runtime/unload", post(unload))
        .route("/runtime/requests/{id}/cancel", post(cancel))
        .route("/runtime/shutdown", post(shutdown))
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            security,
            crate::security::enforce,
        ))
        .layer(axum::Extension(body_limit))
}
/// A distinct router: no management, health, discovery or proof handlers.
pub fn lan_router(state: ApiState, security: Arc<crate::lan::LanSecurityContext>) -> Router {
    let body_limit = crate::security::BodyLimit(state.config.api.max_body_bytes);
    Router::new()
        .route("/v1/models", get(loaded_models))
        .route("/v1/chat/completions", post(crate::chat::lan_chat))
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
        .with_state(state)
        .layer(middleware::from_fn_with_state(
            security,
            crate::lan::enforce,
        ))
        .layer(axum::Extension(body_limit))
}
async fn loaded_models(State(state): State<ApiState>) -> Result<Json<Value>, ApiError> {
    // Observation only; submit_loaded performs the independent atomic admission.
    // LAN reads never compete for the reserved local management control slots.
    let status = state.execute(|runtime| runtime.status()).await?;
    let data: Vec<_> =
        if matches!(status.state, ModelState::Ready | ModelState::Generating) && !status.stopping {
            status
                .selected_model
                .into_iter()
                .map(|id| json!({"id":id,"object":"model","owned_by":"local"}))
                .collect()
        } else {
            Vec::new()
        };
    Ok(Json(json!({"object":"list","data":data})))
}
async fn health() -> Json<Value> {
    Json(json!({"status":"ok"}))
}
async fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "not_found", "Route not found.", None)
}
async fn method_not_allowed() -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "Method not allowed for this route.",
        None,
    )
}
async fn available_models(
    State(state): State<ApiState>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let query = page_query(query)?;
    let (models, next_after, generation) = state.models_page(
        query.limit.unwrap_or(64),
        query.after.as_ref(),
        true,
        query.generation,
    )?;
    let data: Vec<_> = models
        .into_iter()
        .map(|model| json!({"id":model.id,"object":"model","owned_by":"local"}))
        .collect();
    Ok(Json(
        json!({"object":"list","data":data,"next_after":next_after,"generation":generation}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PageQuery {
    limit: Option<usize>,
    after: Option<ModelId>,
    generation: Option<uuid::Uuid>,
}
fn page_query(query: Result<Query<PageQuery>, QueryRejection>) -> Result<PageQuery, ApiError> {
    let Query(query) = query.map_err(|error| {
        let message = error.body_text();
        for marker in ["unknown field `", "duplicate field `"] {
            if let Some(field) = message
                .split(marker)
                .nth(1)
                .and_then(|tail| tail.split('`').next())
            {
                let field: String = field.chars().take(128).collect();
                return ApiError::invalid(&field, "Unknown or duplicate query field.");
            }
        }
        ApiError::invalid(
            "query",
            "Expected limit=1..=128 and an optional model-ID after cursor.",
        )
    })?;
    if !(1..=128).contains(&query.limit.unwrap_or(64)) {
        return Err(ApiError::invalid("limit", "Page size must be 1..=128."));
    }
    Ok(query)
}
async fn models(
    State(state): State<ApiState>,
    query: Result<Query<PageQuery>, QueryRejection>,
) -> Result<Json<Value>, ApiError> {
    let query = page_query(query)?;
    let (data, next_after, generation) = state.models_page(
        query.limit.unwrap_or(64),
        query.after.as_ref(),
        false,
        query.generation,
    )?;
    Ok(Json(
        json!({"object":"list","data":data,"next_after":next_after,"generation":generation}),
    ))
}
fn state_name(state: ModelState) -> &'static str {
    match state {
        ModelState::Unloaded => "unloaded",
        ModelState::Loading => "loading",
        ModelState::Ready => "ready",
        ModelState::Generating => "generating",
        ModelState::Unloading => "unloading",
        ModelState::Faulted => "faulted",
    }
}
pub(crate) fn status_json(state: &ApiState, status: RuntimeStatus) -> Value {
    let diagnostics = state.diagnostics.as_ref();
    let available = std::thread::available_parallelism().ok().map(|n| n.get());
    let threads = status.load_options.map(|options| options.threads);
    json!({
        "lan_api":{"enabled":state.config.lan_api.enabled,"listen":state.config.lan_api.listen,"running":state.lan_is_listening()},
        "state":state_name(status.state), "selected_model_display_name":state.selected_display_name(status.selected_model.as_ref()), "selected_model":status.selected_model,
        "model_library":{"supported":true,"directory":state.model_library_info()},
        "load_options":status.load_options, "active_request":status.active_request,
        "queued_jobs":status.queued_jobs, "stopping":status.stopping, "registry_busy":status.registry_busy,
        "configured_backend":state.config.inference.backend, "backend":null,
        "backend_observation":"unavailable", "last_error":status.last_error.map(ApiError::from).map(|error| error.error),
        "threads_source":state.thread_source(status.load_options),
        "available_parallelism":available,
        "threads_exceed_available_parallelism":threads.zip(available).map(|(threads, available)| threads as usize > available),
        "worker":{"pid":diagnostics.and_then(|d| d.worker_pid()),"sessions_started":diagnostics.map(|d|d.sessions_started()),"sessions_reaped":diagnostics.map(|d|d.sessions_reaped())},
        "memory":{"api_private_bytes":null,"worker_private_bytes":null,"gpu_bytes":null,"observation":"unavailable"}
    })
}
async fn status(State(state): State<ApiState>) -> Result<Json<Value>, ApiError> {
    let status = state.control(|runtime| runtime.status()).await?;
    Ok(Json(status_json(&state, status)))
}
async fn devices() -> Json<Value> {
    Json(
        json!({"build_backends":["cpu"],"devices":[{"id":"cpu","kind":"cpu","logical_processors":std::thread::available_parallelism().ok().map(|n|n.get()),"memory_bytes":null}],"native_device_probe":"unavailable","gpu_devices":null}),
    )
}
pub(crate) async fn json_body(request: Request, max: usize) -> Result<Vec<u8>, ApiError> {
    let content_type = request
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok());
    if !content_type.is_some_and(|value| {
        value
            .split(';')
            .next()
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("application/json"))
    }) {
        return Err(ApiError::invalid(
            "Content-Type",
            "Expected application/json.",
        ));
    }
    to_bytes(request.into_body(), max)
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|_| {
            ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_too_large",
                "Request body exceeds the configured limit or could not be read.",
                None,
            )
        })
}
async fn empty_body(request: Request, max: usize) -> Result<(), ApiError> {
    let bytes = to_bytes(request.into_body(), max).await.map_err(|_| {
        ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request_too_large",
            "Request body exceeds the configured limit.",
            None,
        )
    })?;
    if bytes.is_empty() {
        return Ok(());
    }
    let value = crate::dto::parse_json(&bytes)?;
    match value.as_object() {
        Some(value) if value.is_empty() => Ok(()),
        Some(value) => {
            let field: String = value.keys().next().unwrap().chars().take(128).collect();
            Err(ApiError::invalid(
                &field,
                "This operation accepts no fields.",
            ))
        }
        None => Err(ApiError::invalid(
            "body",
            "This operation accepts no fields.",
        )),
    }
}
async fn import(State(state): State<ApiState>, request: Request) -> Result<Json<Value>, ApiError> {
    let bytes = json_body(request, state.config.api.max_body_bytes).await?;
    let imported = state.import(ImportModelRequest::parse(&bytes)?).await?;
    Ok(Json(
        json!({"model":imported,"id":imported.id,"size_bytes":imported.size_bytes,"sha256":imported.sha256}),
    ))
}
async fn load(State(state): State<ApiState>, request: Request) -> Result<Json<Value>, ApiError> {
    state.ensure_running()?;
    let bytes = json_body(request, state.config.api.max_body_bytes).await?;
    let load = LoadRequest::parse(&bytes)?;
    let options = load.options(&state.active_config()?)?;
    state
        .load(load.model, options, load.threads.is_some())
        .await?;
    let status = state.control(|runtime| runtime.status()).await?;
    Ok(Json(status_json(&state, status)))
}
async fn load_operation_start(
    State(state): State<ApiState>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let bytes = json_body(request, state.config.api.max_body_bytes).await?;
    let mut value = crate::dto::parse_json(&bytes)?;
    let id = value
        .as_object_mut()
        .and_then(|object| object.remove("operation_id"))
        .and_then(|id| id.as_str().and_then(|id| uuid::Uuid::parse_str(id).ok()))
        .filter(|id| !id.is_nil())
        .ok_or_else(|| ApiError::invalid("operation_id", "Expected an opaque operation UUID."))?;
    let only_if_unloaded = match value
        .as_object_mut()
        .and_then(|object| object.remove("only_if_unloaded"))
    {
        None => false,
        Some(Value::Bool(value)) => value,
        _ => return Err(ApiError::invalid("only_if_unloaded", "Expected a boolean.")),
    };
    let load = LoadRequest::parse(&serde_json::to_vec(&value).map_err(|_| ApiError::internal())?)?;
    let options = load.options(&state.active_config()?)?;
    Ok(Json(
        json!({"operation_id":state.load_operation_start(id, load.model, options, only_if_unloaded, load.threads.is_some())?}),
    ))
}
async fn load_operation_next(
    State(state): State<ApiState>,
    id: Result<Path<uuid::Uuid>, PathRejection>,
) -> Result<Json<Value>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::invalid("operation_id", "Invalid operation UUID."))?;
    Ok(Json(state.load_operation_next(id)?))
}
async fn load_operation_cancel(
    State(state): State<ApiState>,
    id: Result<Path<uuid::Uuid>, PathRejection>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::invalid("operation_id", "Invalid operation UUID."))?;
    empty_body(request, state.config.api.max_body_bytes).await?;
    Ok(Json(json!({"stopping":state.load_operation_cancel(id)?})))
}
async fn load_and_test(
    State(state): State<ApiState>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    desktop_load(state, request, false).await
}
async fn load_if_unloaded(
    State(state): State<ApiState>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    desktop_load(state, request, true).await
}
async fn desktop_load(
    state: ApiState,
    request: Request,
    only_if_unloaded: bool,
) -> Result<Json<Value>, ApiError> {
    let bytes = json_body(request, state.config.api.max_body_bytes).await?;
    let load = LoadRequest::parse(&bytes)?;
    let options = load.options(&state.active_config()?)?;
    let observation = state
        .desktop_load(
            load.model,
            options,
            load.threads.is_some(),
            only_if_unloaded,
        )
        .await?;
    let status = state.control(|runtime| runtime.status()).await?;
    let mut value = status_json(&state, status);
    value["local_validation"] =
        serde_json::to_value(observation).map_err(|_| ApiError::internal())?;
    Ok(Json(value))
}
async fn model_test(
    State(state): State<ApiState>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    let bytes = json_body(request, state.config.api.max_body_bytes).await?;
    let load = LoadRequest::parse(&bytes)?;
    let status = state.control(|runtime| runtime.status()).await?;
    let options = if status.selected_model.as_ref() == Some(&load.model)
        && matches!(status.state, ModelState::Ready | ModelState::Generating)
    {
        let actual = status.load_options.ok_or_else(ApiError::internal)?;
        let mut validation = state.active_config()?;
        validation.model_profiles.clear();
        validation.inference.context_size = actual.context_size;
        validation.inference.threads = Some(actual.threads);
        validation.inference.batch_size = actual.batch_size;
        load.options(&validation)?; // Explicit invalid fields must still fail.
        actual
    } else {
        load.options(&state.active_config()?)?
    };
    Ok(Json(
        serde_json::to_value(state.model_probe(load.model, options).await?)
            .map_err(|_| ApiError::internal())?,
    ))
}
async fn unload(State(state): State<ApiState>, request: Request) -> Result<Json<Value>, ApiError> {
    state.ensure_running()?;
    empty_body(request, state.config.api.max_body_bytes).await?;
    state.execute(|runtime| runtime.unload()).await?;
    let status = state.control(|runtime| runtime.status()).await?;
    Ok(Json(status_json(&state, status)))
}
async fn cancel(
    State(state): State<ApiState>,
    id: Result<Path<String>, PathRejection>,
    request: Request,
) -> Result<Response, ApiError> {
    empty_body(request, state.config.api.max_body_bytes).await?;
    let Path(id) = id.map_err(|_| ApiError::invalid("id", "Request ID must be a UUID."))?;
    let id: RequestId = id
        .parse()
        .map_err(|_| ApiError::invalid("id", "Request ID must be a UUID."))?;
    state.control(move |runtime| runtime.cancel(id)).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"request_id":id,"status":"cancelling"})),
    )
        .into_response())
}
async fn shutdown(
    State(state): State<ApiState>,
    request: Request,
) -> Result<Json<Value>, ApiError> {
    empty_body(request, state.config.api.max_body_bytes).await?;
    state.shutdown.begin();
    state.wait_shutdown().await?;
    Ok(Json(json!({"status":"stopped"})))
}

async fn configuration_get(
    State(state): State<ApiState>,
) -> Result<Json<crate::configuration::ConfigurationSnapshot>, ApiError> {
    Ok(Json(state.configuration_get().await?))
}
async fn configuration_save(
    State(state): State<ApiState>,
    request: Request,
) -> Result<Json<crate::configuration::ConfigurationSnapshot>, ApiError> {
    let bytes = json_body(request, state.config.api.max_body_bytes).await?;
    let request = crate::configuration::parse_save_request(&bytes)?;
    Ok(Json(state.configuration_save(request).await?))
}
async fn configuration_model_get(
    State(state): State<ApiState>,
    id: Result<Path<ModelId>, PathRejection>,
) -> Result<Json<crate::configuration::ModelConfiguration>, ApiError> {
    let Path(id) = id.map_err(|_| ApiError::invalid("model_id", "Invalid model ID."))?;
    Ok(Json(state.configuration_model_get(id).await?))
}

async fn unregister(
    State(state): State<ApiState>,
    request: Request,
) -> Result<Json<crate::dto::UnregisterModelResult>, ApiError> {
    let bytes = json_body(request, state.config.api.max_body_bytes).await?;
    Ok(Json(
        state
            .unregister(crate::dto::parse_unregister(&bytes)?)
            .await?,
    ))
}
