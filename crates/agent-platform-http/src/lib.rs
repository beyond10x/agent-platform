#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use agent_platform_api::{
    ACTIVATE_PATH, AGENT_PATH, AGENTS_PATH, CAPABILITY_PROFILE_PATH, CAPABILITY_PROFILES_PATH,
    DOCS_API_PATH, DOCS_INDEX_PATH, DOCS_ROOT_PATH, DOCS_STYLES_PATH, LIVENESS_PATH, OPENAPI_PATH,
    ProblemDocument, REVISIONS_PATH, TASK_APPROVAL_PATH, TASK_APPROVALS_PATH, TASK_EVENTS_PATH,
    TASK_PATH, TASKS_PATH, TRIGGERS_PATH,
};
use agent_platform_app::{Application, ApplicationError, TaskExecutionPlan, TrustedRequestContext};
use agent_platform_auth::{AttemptConnectorAccess, CredentialVerifier, UserModelLease, operation};
use agent_platform_core::{
    ActivateRevision, AgentId, ApprovalId, AttemptId, CapabilityProfileId, ConnectorOwnerContext,
    CreateAgent, CreateCapabilityProfile, CreateTrigger, PendingApproval, RequestId,
    ResolveApproval, RevisionSpec, SubmitTask, TaskEventKind, TaskFailure, TaskId, TenantId,
    UpdateCapabilityProfile,
};
use agent_platform_harness::{
    ApprovalDecision, ApprovalPort, ConnectorApprovalEvidence, UserModelExecution, UserModelRunner,
};
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Extension, Path, State};
use axum::http::{Request, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

const MAX_HTTP_BODY_BYTES: usize = 512 * 1024;
static OPENAPI: LazyLock<Bytes> =
    LazyLock::new(|| Bytes::from(agent_platform_openapi::document_bytes()));

#[derive(Clone)]
pub struct HttpState {
    app: Application,
    verifier: Arc<dyn CredentialVerifier>,
    runner: Option<UserModelRunner>,
    approvals: Arc<ApprovalRegistry>,
}

impl HttpState {
    pub fn new(app: Application, verifier: Arc<dyn CredentialVerifier>) -> Self {
        Self {
            app,
            verifier,
            runner: None,
            approvals: Arc::new(ApprovalRegistry::default()),
        }
    }

    #[must_use]
    pub fn with_runner(mut self, runner: UserModelRunner) -> Self {
        self.runner = Some(runner);
        self
    }
}

#[derive(Clone)]
struct AttemptAdmission {
    attempt_id: AttemptId,
    lease: Option<Arc<UserModelLease>>,
    connector_access: Option<Arc<AttemptConnectorAccess>>,
}

#[derive(Debug, Clone)]
struct ApprovalTarget {
    operation: String,
    connection: String,
    description: String,
}

#[derive(Debug)]
struct ApprovalEntry {
    approval: PendingApproval,
    resolution: Option<ResolveApproval>,
}

#[derive(Debug, Default)]
struct ApprovalRegistry {
    entries: Mutex<BTreeMap<ApprovalId, ApprovalEntry>>,
    changed: Condvar,
}

impl ApprovalRegistry {
    fn publish_and_wait(
        &self,
        approval: PendingApproval,
        publish: impl FnOnce() -> Result<(), ()>,
    ) -> Result<ResolveApproval, ()> {
        let approval_id = approval.id.clone();
        let mut entries = self.entries.lock().map_err(|_| ())?;
        if entries
            .insert(
                approval_id.clone(),
                ApprovalEntry {
                    approval,
                    resolution: None,
                },
            )
            .is_some()
        {
            return Err(());
        }
        if publish().is_err() {
            entries.remove(&approval_id);
            return Err(());
        }
        let deadline = std::time::Instant::now() + Duration::from_mins(30);
        loop {
            if let Some(resolution) = entries
                .get(&approval_id)
                .and_then(|entry| entry.resolution.clone())
            {
                entries.remove(&approval_id);
                return Ok(resolution);
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                entries.remove(&approval_id);
                return Err(());
            }
            let remaining = deadline.saturating_duration_since(now);
            let (next, timeout) = self
                .changed
                .wait_timeout(entries, remaining)
                .map_err(|_| ())?;
            entries = next;
            if timeout.timed_out() {
                entries.remove(&approval_id);
                return Err(());
            }
        }
    }

    fn list(&self, task_id: &TaskId) -> Result<Vec<PendingApproval>, ()> {
        let entries = self.entries.lock().map_err(|_| ())?;
        Ok(entries
            .values()
            .filter(|entry| &entry.approval.task_id == task_id && entry.resolution.is_none())
            .map(|entry| entry.approval.clone())
            .collect())
    }

    fn resolve(
        &self,
        task_id: &TaskId,
        attempt_id: &AttemptId,
        approval_id: &ApprovalId,
        resolution: ResolveApproval,
    ) -> Result<PendingApproval, ()> {
        let mut entries = self.entries.lock().map_err(|_| ())?;
        let entry = entries.get_mut(approval_id).ok_or(())?;
        if &entry.approval.task_id != task_id
            || &entry.approval.attempt_id != attempt_id
            || entry.resolution.is_some()
        {
            return Err(());
        }
        let approval = entry.approval.clone();
        entry.resolution = Some(resolution);
        self.changed.notify_all();
        Ok(approval)
    }
}

struct AttemptApprover {
    app: Application,
    registry: Arc<ApprovalRegistry>,
    evidence: ConnectorApprovalEvidence,
    tenant_id: TenantId,
    task_id: TaskId,
    attempt_id: AttemptId,
    context: ConnectorOwnerContext,
    targets: BTreeMap<String, ApprovalTarget>,
}

impl ApprovalPort for AttemptApprover {
    fn decide(
        &mut self,
        call: &harness_wire::ToolCall,
        spec: &harness_wire::ToolSpec,
    ) -> ApprovalDecision {
        let Some(target) = self.targets.get(spec.name.as_str()).cloned() else {
            return ApprovalDecision::denied("the approved tool has no compiled Connector target");
        };
        let Ok(approval_id) = ApprovalId::new(format!("apr_{}", Uuid::now_v7().simple())) else {
            return ApprovalDecision::denied("approval identity is unavailable");
        };
        let call_id = call.call_id.as_str().to_owned();
        let requested_at_ms = now_ms();
        let pending = PendingApproval {
            id: approval_id.clone(),
            task_id: self.task_id.clone(),
            attempt_id: self.attempt_id.clone(),
            call_id: call_id.clone(),
            tool_name: spec.name.as_str().to_owned(),
            operation_ref: target.operation.clone(),
            connection_ref: target.connection.clone(),
            description_ref: target.description,
            input: call.arguments.clone(),
            context: self.context.clone(),
            requested_at_ms,
        };
        let resolution = self.registry.publish_and_wait(pending.clone(), || {
            self.app
                .mark_task_awaiting_approval(&self.tenant_id, &pending)
                .map_err(|_| ())
        });
        let approved = matches!(resolution, Ok(ResolveApproval::Approve { .. }));
        let _ = self.app.resolve_task_approval(
            &self.tenant_id,
            &self.task_id,
            &self.attempt_id,
            now_ms(),
            approval_id,
            approved,
        );
        match resolution {
            Ok(ResolveApproval::Approve {
                approval_evidence_ref,
            }) => match self.evidence.approve(call_id, approval_evidence_ref) {
                Ok(()) => ApprovalDecision::Approved,
                Err(_) => ApprovalDecision::denied("approval evidence handoff is unavailable"),
            },
            Ok(ResolveApproval::Deny { reason }) => ApprovalDecision::denied(reason),
            Err(()) => ApprovalDecision::denied("the human approval request expired"),
        }
    }
}

pub fn router(state: HttpState) -> Router {
    let protected = Router::new()
        .route(AGENTS_PATH, get(list_agents).post(create_agent))
        .route(AGENT_PATH, get(get_agent))
        .route(REVISIONS_PATH, get(list_revisions).post(create_revision))
        .route(ACTIVATE_PATH, post(activate_revision))
        .route(
            CAPABILITY_PROFILES_PATH,
            get(list_capability_profiles).post(create_capability_profile),
        )
        .route(CAPABILITY_PROFILE_PATH, patch(update_capability_profile))
        .route(TASKS_PATH, get(list_tasks).post(submit_task))
        .route(TASK_PATH, get(get_task))
        .route(TASK_EVENTS_PATH, get(stream_task_events))
        .route(TASK_APPROVALS_PATH, get(list_task_approvals))
        .route(TASK_APPROVAL_PATH, post(resolve_task_approval))
        .route(TRIGGERS_PATH, get(list_triggers).post(create_trigger))
        .route_layer(middleware::from_fn_with_state(state.clone(), authenticate));

    Router::new()
        .route(LIVENESS_PATH, get(liveness))
        .route(OPENAPI_PATH, get(openapi))
        .route(
            DOCS_ROOT_PATH,
            get(|| async { Redirect::permanent(DOCS_INDEX_PATH) }),
        )
        .route(DOCS_INDEX_PATH, get(docs_index))
        .route(DOCS_API_PATH, get(docs_api))
        .route(DOCS_STYLES_PATH, get(docs_styles))
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
    let attempt_id = (request.method() == axum::http::Method::POST
        && request.uri().path() == TASKS_PATH)
        .then(new_attempt_id)
        .transpose();
    let attempt_id = match attempt_id {
        Ok(attempt_id) => attempt_id,
        Err(error) => {
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "attempt_identity_unavailable",
                &error,
            );
        }
    };
    let authenticated = match state
        .verifier
        .verify(authorization, attempt_id.as_ref())
        .await
    {
        Ok(authenticated) => authenticated,
        Err(error) => return problem(StatusCode::UNAUTHORIZED, "unauthenticated", error.reason()),
    };
    let (authority, lease, connector_access) = authenticated.into_parts();
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
    if let Some(attempt_id) = attempt_id {
        request.extensions_mut().insert(AttemptAdmission {
            attempt_id,
            lease,
            connector_access,
        });
    }
    next.run(request).await
}

async fn liveness() -> &'static str {
    "ok\n"
}

async fn openapi() -> Response {
    public_response(
        OPENAPI.clone(),
        "application/json; charset=utf-8",
        "no-store",
        false,
    )
}

async fn docs_index() -> Response {
    embedded_docs("index")
}

async fn docs_api() -> Response {
    embedded_docs("api")
}

async fn docs_styles() -> Response {
    embedded_docs("styles")
}

fn embedded_docs(name: &str) -> Response {
    let Some(asset) = agent_platform_docs::asset(name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    public_response(
        Bytes::from_static(asset.bytes),
        asset.content_type,
        asset.cache_control,
        asset.content_type.starts_with("text/html"),
    )
}

fn public_response(
    bytes: Bytes,
    content_type: &'static str,
    cache_control: &'static str,
    html: bool,
) -> Response {
    let mut response = Response::new(Body::from(bytes));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static(content_type),
    );
    headers.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static(cache_control),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    if html {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            header::HeaderValue::from_static(
                "default-src 'none'; style-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
            ),
        );
    }
    response
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

async fn update_capability_profile(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path(profile_id): Path<String>,
    Json(request): Json<UpdateCapabilityProfile>,
) -> Response {
    let profile_id = match CapabilityProfileId::new(profile_id) {
        Ok(profile_id) => profile_id,
        Err(error) => return application_error(&ApplicationError::Invalid(error)),
    };
    result(
        StatusCode::OK,
        state
            .app
            .update_capability_profile(&context, &profile_id, request)
            .await,
    )
}

async fn submit_task(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Extension(attempt): Extension<AttemptAdmission>,
    Json(request): Json<SubmitTask>,
) -> Response {
    let admission = match state
        .app
        .admit_task(&context, request, attempt.attempt_id.clone())
    {
        Ok(admission) => admission,
        Err(error) => return application_error(&error),
    };
    if admission.newly_created
        && let (Some(runner), Some(lease)) = (state.runner.clone(), attempt.lease)
    {
        spawn_execution(
            state.app.clone(),
            runner,
            admission.plan.clone(),
            lease,
            attempt.connector_access,
            state.approvals.clone(),
        );
    }
    (StatusCode::ACCEPTED, Json(admission.plan.task)).into_response()
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

async fn list_task_approvals(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path(task_id): Path<String>,
) -> Response {
    let task_id = match TaskId::new(task_id) {
        Ok(task_id) => task_id,
        Err(error) => return application_error(&ApplicationError::Invalid(error)),
    };
    if let Err(error) = state.app.get_task_for_approval(&context, &task_id) {
        return application_error(&error);
    }
    match state.approvals.list(&task_id) {
        Ok(approvals) => (StatusCode::OK, Json(approvals)).into_response(),
        Err(()) => problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "approval_state_unavailable",
            "task approval state is unavailable",
        ),
    }
}

async fn resolve_task_approval(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path((task_id, approval_id)): Path<(String, String)>,
    Json(resolution): Json<ResolveApproval>,
) -> Response {
    let task_id = match TaskId::new(task_id) {
        Ok(task_id) => task_id,
        Err(error) => return application_error(&ApplicationError::Invalid(error)),
    };
    let approval_id = match ApprovalId::new(approval_id) {
        Ok(approval_id) => approval_id,
        Err(error) => return application_error(&ApplicationError::Invalid(error)),
    };
    if let Err(error) = resolution.validate() {
        return application_error(&ApplicationError::Invalid(error));
    }
    let task = match state.app.get_task_for_approval(&context, &task_id) {
        Ok(task) => task,
        Err(error) => return application_error(&error),
    };
    if task.status != agent_platform_core::TaskStatus::AwaitingApproval {
        return problem(
            StatusCode::CONFLICT,
            "approval_not_pending",
            "the task is not awaiting approval",
        );
    }
    match state
        .approvals
        .resolve(&task_id, &task.attempt_id, &approval_id, resolution)
    {
        Ok(approval) => (StatusCode::ACCEPTED, Json(approval)).into_response(),
        Err(()) => problem(
            StatusCode::NOT_FOUND,
            "approval_not_found",
            "the pending approval was not found",
        ),
    }
}

async fn stream_task_events(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path(task_id): Path<String>,
) -> Response {
    let task_id = match TaskId::new(task_id) {
        Ok(task_id) => task_id,
        Err(error) => return application_error(&ApplicationError::Invalid(error)),
    };
    let subscription = match state.app.subscribe_task_events(&context, &task_id) {
        Ok(subscription) => subscription,
        Err(error) => return application_error(&error),
    };
    let mut receiver = subscription.receiver;
    let events = async_stream::stream! {
        let mut terminal = false;
        let mut last_sequence = 0;
        for event in subscription.backlog {
            terminal = matches!(event.event, TaskEventKind::Succeeded { .. } | TaskEventKind::Failed { .. });
            last_sequence = event.sequence;
            let id = event.sequence.to_string();
            match Event::default().id(id).event("task").json_data(event) {
                Ok(event) => yield Ok::<Event, std::convert::Infallible>(event),
                Err(_) => return,
            }
        }
        while !terminal {
            match receiver.recv().await {
                Ok(event) => {
                    if event.sequence <= last_sequence {
                        continue;
                    }
                    terminal = matches!(event.event, TaskEventKind::Succeeded { .. } | TaskEventKind::Failed { .. });
                    last_sequence = event.sequence;
                    let id = event.sequence.to_string();
                    match Event::default().id(id).event("task").json_data(event) {
                        Ok(event) => yield Ok::<Event, std::convert::Infallible>(event),
                        Err(_) => return,
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let Ok(recovered) = state
                        .app
                        .task_events_after(&context, &task_id, last_sequence)
                    else {
                        return;
                    };
                    for event in recovered {
                        terminal = matches!(event.event, TaskEventKind::Succeeded { .. } | TaskEventKind::Failed { .. });
                        last_sequence = event.sequence;
                        let id = event.sequence.to_string();
                        match Event::default().id(id).event("task").json_data(event) {
                            Ok(event) => yield Ok::<Event, std::convert::Infallible>(event),
                            Err(_) => return,
                        }
                    }
                },
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(events)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
        .into_response()
}

fn spawn_execution(
    app: Application,
    runner: UserModelRunner,
    plan: TaskExecutionPlan,
    lease: Arc<UserModelLease>,
    connector_access: Option<Arc<AttemptConnectorAccess>>,
    approvals: Arc<ApprovalRegistry>,
) {
    tokio::spawn(async move {
        let tenant_id = plan.task.tenant_id.clone();
        let task_id = plan.task.id.clone();
        let attempt_id = plan.task.attempt_id.clone();
        if app
            .mark_task_running(&tenant_id, &task_id, &attempt_id, now_ms())
            .is_err()
        {
            return;
        }
        let event_app = app.clone();
        let event_tenant = tenant_id.clone();
        let event_task = task_id.clone();
        let event_attempt = attempt_id.clone();
        let emit = Arc::new(move |text: String| {
            let _ = event_app.append_task_text(
                &event_tenant,
                &event_task,
                &event_attempt,
                now_ms(),
                text,
            );
        });
        let approval_evidence = ConnectorApprovalEvidence::default();
        let targets = plan.toolset.as_ref().map_or_else(BTreeMap::new, |toolset| {
            toolset
                .capabilities
                .iter()
                .map(|capability| {
                    (
                        capability.tool.name.to_string(),
                        ApprovalTarget {
                            operation: capability.operation_ref.clone(),
                            connection: capability.connection_ref.clone(),
                            description: capability.description_ref.clone(),
                        },
                    )
                })
                .collect()
        });
        let approval_context = ConnectorOwnerContext {
            tenant_id: tenant_id.clone(),
            agent_id: plan.task.agent_id.clone(),
            agent_revision: plan.task.agent_revision,
            authority_snapshot_id: plan.task.request_id.clone(),
            authority_snapshot_sha256: authority_snapshot_sha256(&plan),
        };
        let approver = AttemptApprover {
            app: app.clone(),
            registry: approvals,
            evidence: approval_evidence.clone(),
            tenant_id: tenant_id.clone(),
            task_id: task_id.clone(),
            attempt_id: attempt_id.clone(),
            context: approval_context.clone(),
            targets,
        };
        let connector_context = operation::OwnerContext {
            tenant_id: approval_context.tenant_id.to_string(),
            agent_id: approval_context.agent_id.to_string(),
            agent_revision: approval_context.agent_revision,
            authority_snapshot_id: approval_context.authority_snapshot_id.to_string(),
            authority_snapshot_sha256: approval_context.authority_snapshot_sha256,
        };
        match runner
            .execute(
                UserModelExecution {
                    revision: plan.revision,
                    toolset: plan.toolset,
                    input: plan.task.input,
                    lease,
                    connector_access,
                    attempt_id: attempt_id.clone(),
                    connector_context,
                    approval_evidence,
                    approvals: Box::new(approver),
                },
                emit,
            )
            .await
        {
            Ok(output) => {
                let _ = app.succeed_task(&tenant_id, &task_id, &attempt_id, now_ms(), output);
            }
            Err(error) => {
                let _ = app.fail_task(
                    &tenant_id,
                    &task_id,
                    &attempt_id,
                    now_ms(),
                    TaskFailure {
                        code: error.code().to_owned(),
                        message: error.to_string(),
                    },
                );
            }
        }
    });
}

fn authority_snapshot_sha256(plan: &TaskExecutionPlan) -> String {
    let mut digest = Sha256::new();
    for value in [
        plan.task.tenant_id.as_str(),
        plan.task.agent_id.as_str(),
        &plan.task.agent_revision.to_string(),
        plan.task.actor.as_str(),
        plan.task
            .executor
            .as_ref()
            .map_or("", agent_platform_core::SubjectId::as_str),
        plan.task
            .delegation_id
            .as_ref()
            .map_or("", agent_platform_core::DelegationId::as_str),
        plan.task.request_id.as_str(),
    ] {
        digest.update(value.len().to_be_bytes());
        digest.update(value.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn new_attempt_id() -> Result<AttemptId, String> {
    AttemptId::new(format!("atm_{}", Uuid::now_v7().simple())).map_err(|error| error.to_string())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
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
        ApplicationError::ActiveRevisionConflict { .. }
        | ApplicationError::CapabilityProfileRevisionConflict { .. }
        | ApplicationError::IdempotencyConflict => (StatusCode::CONFLICT, "conflict"),
        ApplicationError::NoActiveRevision | ApplicationError::Invalid(_) => {
            (StatusCode::UNPROCESSABLE_ENTITY, "invalid_request")
        }
        ApplicationError::Projection(_) => (StatusCode::UNPROCESSABLE_ENTITY, "capability_refused"),
        ApplicationError::StateUnavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        ApplicationError::StatePersistence => (
            StatusCode::SERVICE_UNAVAILABLE,
            "state_persistence_unavailable",
        ),
    };
    problem(status, code, &error.to_string())
}

fn problem(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(ProblemDocument {
            code: code.to_owned(),
            message: message.to_owned(),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_platform_api::{Method as ApiMethod, ROUTES};
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

    async fn bytes(response: Response) -> Bytes {
        response.into_body().collect().await.unwrap().to_bytes()
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
    async fn route_catalog_and_authenticated_router_cannot_drift_apart() {
        for route in ROUTES.iter().filter(|route| route.authenticated) {
            let path = route
                .path
                .replace("{agent_id}", "agent-one")
                .replace("{profile_id}", "profile-one")
                .replace("{task_id}", "task-one")
                .replace("{approval_id}", "approval-one");
            let method = match route.method {
                ApiMethod::Get => Method::GET,
                ApiMethod::Post => Method::POST,
                ApiMethod::Patch => Method::PATCH,
            };
            let response = service()
                .oneshot(request(method, &path, "not json", None))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{} {} was not registered behind authentication",
                route.method.as_str(),
                route.path
            );
        }
    }

    #[tokio::test]
    async fn generated_openapi_and_embedded_docs_are_public_exact_assets() {
        let openapi = service()
            .oneshot(request(Method::GET, OPENAPI_PATH, "", None))
            .await
            .unwrap();
        assert_eq!(openapi.status(), StatusCode::OK);
        assert_eq!(openapi.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(openapi.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(
            bytes(openapi).await,
            agent_platform_openapi::document_bytes()
        );

        let docs = service()
            .oneshot(request(Method::GET, DOCS_INDEX_PATH, "", None))
            .await
            .unwrap();
        assert_eq!(docs.status(), StatusCode::OK);
        assert_eq!(
            docs.headers()[header::CONTENT_SECURITY_POLICY],
            "default-src 'none'; style-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
        );
        let docs = String::from_utf8(bytes(docs).await.to_vec()).unwrap();
        assert!(docs.contains("Agents with a small,"));
        assert!(docs.contains(OPENAPI_PATH));
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

    #[test]
    fn pending_approval_is_published_exactly_and_resolved_once() {
        let registry = Arc::new(ApprovalRegistry::default());
        let approval = PendingApproval {
            id: ApprovalId::new("approval-one").unwrap(),
            task_id: TaskId::new("task-one").unwrap(),
            attempt_id: AttemptId::new("attempt-one").unwrap(),
            call_id: "call-one".to_owned(),
            tool_name: "todo_create_item".to_owned(),
            operation_ref: "todo.item.create".to_owned(),
            connection_ref: "todo".to_owned(),
            description_ref: "description-one".to_owned(),
            input: json!({"list_id": "list-one", "title": "Ship it"}),
            context: ConnectorOwnerContext {
                tenant_id: TenantId::new("tenant-one").unwrap(),
                agent_id: AgentId::new("agent-one").unwrap(),
                agent_revision: 1,
                authority_snapshot_id: RequestId::new("request-one").unwrap(),
                authority_snapshot_sha256: "a".repeat(64),
            },
            requested_at_ms: 42,
        };
        let (published_tx, published_rx) = std::sync::mpsc::channel();
        let worker_registry = registry.clone();
        let worker_approval = approval.clone();
        let worker = std::thread::spawn(move || {
            worker_registry
                .publish_and_wait(worker_approval, || published_tx.send(()).map_err(|_| ()))
        });
        published_rx.recv().unwrap();

        assert_eq!(
            registry.list(&approval.task_id).unwrap(),
            vec![approval.clone()]
        );
        let decision = ResolveApproval::Approve {
            approval_evidence_ref: "approval-proof-one".to_owned(),
        };
        assert_eq!(
            registry
                .resolve(
                    &approval.task_id,
                    &approval.attempt_id,
                    &approval.id,
                    decision.clone(),
                )
                .unwrap(),
            approval
        );
        assert_eq!(worker.join().unwrap().unwrap(), decision);
        assert!(
            registry
                .list(&TaskId::new("task-one").unwrap())
                .unwrap()
                .is_empty()
        );
    }
}
