#![forbid(unsafe_code)]

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use agent_platform_app::{Application, ApplicationError, TrustedRequestContext};
use agent_platform_auth::CredentialVerifier;
use agent_platform_core::{
    ActivateRevision, AgentId, CreateAgent, CreateCapabilityProfile, CreateTrigger, RequestId,
    RevisionSpec, SubmitTask, TaskId,
};
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Extension, Path, State};
use axum::http::{Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use uuid::Uuid;

const MAX_HTTP_BODY_BYTES: usize = 512 * 1024;

#[derive(Clone)]
pub struct HttpState {
    app: Application,
    verifier: Arc<dyn CredentialVerifier>,
}

impl HttpState {
    pub fn new(app: Application, verifier: Arc<dyn CredentialVerifier>) -> Self {
        Self { app, verifier }
    }
}

pub fn router(state: HttpState) -> Router {
    let protected = Router::new()
        .route("/v1/agents", get(list_agents).post(create_agent))
        .route("/v1/agents/{agent_id}", get(get_agent))
        .route(
            "/v1/agents/{agent_id}/revisions",
            get(list_revisions).post(create_revision),
        )
        .route("/v1/agents/{agent_id}/activate", post(activate_revision))
        .route(
            "/v1/capability-profiles",
            get(list_capability_profiles).post(create_capability_profile),
        )
        .route("/v1/tasks", get(list_tasks).post(submit_task))
        .route("/v1/tasks/{task_id}", get(get_task))
        .route("/v1/triggers", get(list_triggers).post(create_trigger))
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate));

    Router::new()
        .route("/livez", get(liveness))
        .merge(protected)
        .layer(DefaultBodyLimit::max(MAX_HTTP_BODY_BYTES))
        .with_state(state)
}

async fn authenticate(
    State(state): State<HttpState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let authorization = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let authority = match state.verifier.verify(authorization).await {
        Ok(authority) => authority,
        Err(error) => return problem(StatusCode::UNAUTHORIZED, "unauthenticated", error.reason()),
    };
    let request_id = match RequestId::new(format!("req_{}", Uuid::now_v7().simple())) {
        Ok(request_id) => request_id,
        Err(error) => {
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "request_identity_unavailable",
                &error.to_string(),
            );
        }
    };
    let received_at_ms = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        Err(_) => {
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "clock_unavailable",
                "server time is before the Unix epoch",
            );
        }
    };
    request.extensions_mut().insert(TrustedRequestContext::new(
        authority,
        request_id,
        received_at_ms,
    ));
    next.run(request).await
}

async fn liveness() -> &'static str {
    "ok\n"
}

async fn create_agent(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Json(request): Json<CreateAgent>,
) -> Response {
    result(
        StatusCode::CREATED,
        state.app.create_agent(&context, request),
    )
}

async fn list_agents(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
) -> Response {
    result(StatusCode::OK, state.app.list_agents(&context))
}

async fn get_agent(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path(agent_id): Path<String>,
) -> Response {
    let agent_id = match AgentId::new(agent_id) {
        Ok(agent_id) => agent_id,
        Err(error) => return application_error(&ApplicationError::Invalid(error)),
    };
    result(StatusCode::OK, state.app.get_agent(&context, &agent_id))
}

async fn create_revision(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path(agent_id): Path<String>,
    Json(request): Json<RevisionSpec>,
) -> Response {
    let agent_id = match AgentId::new(agent_id) {
        Ok(agent_id) => agent_id,
        Err(error) => return application_error(&ApplicationError::Invalid(error)),
    };
    result(
        StatusCode::CREATED,
        state.app.create_revision(&context, &agent_id, request),
    )
}

async fn list_revisions(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path(agent_id): Path<String>,
) -> Response {
    let agent_id = match AgentId::new(agent_id) {
        Ok(agent_id) => agent_id,
        Err(error) => return application_error(&ApplicationError::Invalid(error)),
    };
    result(
        StatusCode::OK,
        state.app.list_revisions(&context, &agent_id),
    )
}

async fn activate_revision(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path(agent_id): Path<String>,
    Json(request): Json<ActivateRevision>,
) -> Response {
    let agent_id = match AgentId::new(agent_id) {
        Ok(agent_id) => agent_id,
        Err(error) => return application_error(&ApplicationError::Invalid(error)),
    };
    result(
        StatusCode::OK,
        state.app.activate_revision(&context, &agent_id, &request),
    )
}

async fn create_capability_profile(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Json(request): Json<CreateCapabilityProfile>,
) -> Response {
    result(
        StatusCode::CREATED,
        state.app.create_capability_profile(&context, request).await,
    )
}

async fn list_capability_profiles(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
) -> Response {
    result(StatusCode::OK, state.app.list_capability_profiles(&context))
}

async fn submit_task(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Json(request): Json<SubmitTask>,
) -> Response {
    result(
        StatusCode::ACCEPTED,
        state.app.submit_task(&context, request),
    )
}

async fn list_tasks(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
) -> Response {
    result(StatusCode::OK, state.app.list_tasks(&context))
}

async fn get_task(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path(task_id): Path<String>,
) -> Response {
    let task_id = match TaskId::new(task_id) {
        Ok(task_id) => task_id,
        Err(error) => return application_error(&ApplicationError::Invalid(error)),
    };
    result(StatusCode::OK, state.app.get_task(&context, &task_id))
}

async fn create_trigger(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Json(request): Json<CreateTrigger>,
) -> Response {
    result(
        StatusCode::CREATED,
        state.app.create_trigger(&context, request),
    )
}

async fn list_triggers(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
) -> Response {
    result(StatusCode::OK, state.app.list_triggers(&context))
}

fn result<T: Serialize>(status: StatusCode, result: Result<T, ApplicationError>) -> Response {
    match result {
        Ok(value) => (status, Json(value)).into_response(),
        Err(error) => application_error(&error),
    }
}

fn application_error(error: &ApplicationError) -> Response {
    let (status, code) = match error {
        ApplicationError::Forbidden { .. } => (StatusCode::FORBIDDEN, "forbidden"),
        ApplicationError::AgentNotFound
        | ApplicationError::RevisionNotFound
        | ApplicationError::CapabilityProfileNotFound
        | ApplicationError::TaskNotFound => (StatusCode::NOT_FOUND, "not_found"),
        ApplicationError::ActiveRevisionConflict { .. } | ApplicationError::IdempotencyConflict => {
            (StatusCode::CONFLICT, "conflict")
        }
        ApplicationError::NoActiveRevision | ApplicationError::Invalid(_) => {
            (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request")
        }
        ApplicationError::Projection(_) => (StatusCode::UNPROCESSABLE_ENTITY, "capability_refused"),
        ApplicationError::StateUnavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
    };
    problem(status, code, &error.to_string())
}

#[derive(Debug, Serialize)]
struct Problem<'a> {
    code: &'a str,
    message: &'a str,
}

fn problem(status: StatusCode, code: &str, message: &str) -> Response {
    (status, Json(Problem { code, message })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_platform_auth::{
        AGENTS_MANAGE, AGENTS_READ, CAPABILITIES_MANAGE, CAPABILITIES_READ, DevelopmentVerifier,
        TASKS_READ, TASKS_SUBMIT, TRIGGERS_MANAGE, TRIGGERS_READ, VerifiedAuthority,
    };
    use agent_platform_connectors::EmptyCatalog;
    use agent_platform_core::{SubjectId, TenantId};
    use axum::http::Method;
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    fn service() -> Router {
        let scopes = [
            AGENTS_MANAGE,
            AGENTS_READ,
            CAPABILITIES_MANAGE,
            CAPABILITIES_READ,
            TASKS_READ,
            TASKS_SUBMIT,
            TRIGGERS_MANAGE,
            TRIGGERS_READ,
        ]
        .into_iter()
        .map(str::to_owned);
        let authority = VerifiedAuthority::new(
            TenantId::new("tenant-one").unwrap(),
            SubjectId::new("human-alice").unwrap(),
            None,
            None,
            scopes,
        )
        .unwrap();
        let verifier = DevelopmentVerifier::new("a-development-secret", authority).unwrap();
        router(HttpState::new(
            Application::new(Arc::new(EmptyCatalog)),
            Arc::new(verifier),
        ))
    }

    fn request(method: Method, path: &str, body: &str, token: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        builder.body(Body::from(body.to_owned())).unwrap()
    }

    async fn json_body(response: Response) -> Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn authentication_precedes_json_materialization() {
        let response = service()
            .oneshot(request(Method::POST, "/v1/agents", "not json", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(json_body(response).await["code"], "unauthenticated");
    }

    #[tokio::test]
    async fn api_creates_activates_and_pins_a_task_revision() {
        let app = service();
        let created = app
            .clone()
            .oneshot(request(
                Method::POST,
                "/v1/agents",
                r#"{"name":"Support helper"}"#,
                Some("a-development-secret"),
            ))
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let agent = json_body(created).await;
        let agent_id = agent["id"].as_str().unwrap();

        let revision = app
            .clone()
            .oneshot(request(
                Method::POST,
                &format!("/v1/agents/{agent_id}/revisions"),
                r#"{"instructions":"Help with support.","model":"model-one"}"#,
                Some("a-development-secret"),
            ))
            .await
            .unwrap();
        assert_eq!(revision.status(), StatusCode::CREATED);

        let activated = app
            .clone()
            .oneshot(request(
                Method::POST,
                &format!("/v1/agents/{agent_id}/activate"),
                r#"{"revision":1,"expected_active_revision":null}"#,
                Some("a-development-secret"),
            ))
            .await
            .unwrap();
        assert_eq!(activated.status(), StatusCode::OK);

        let task = app
            .oneshot(request(
                Method::POST,
                "/v1/tasks",
                &json!({
                    "agent_id": agent_id,
                    "idempotency_key": "client-attempt-one",
                    "input": {"prompt": "hello"}
                })
                .to_string(),
                Some("a-development-secret"),
            ))
            .await
            .unwrap();
        assert_eq!(task.status(), StatusCode::ACCEPTED);
        let task = json_body(task).await;
        assert_eq!(task["agent_revision"], 1);
        assert_eq!(task["status"], "accepted");
        assert_eq!(task["actor"], "human-alice");
    }
}
