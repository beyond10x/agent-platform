#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use agent_platform_api::{
    ACTIVATE_PATH, AGENT_PATH, AGENTS_PATH, CAPABILITY_PROFILE_PATH, CAPABILITY_PROFILES_PATH,
    CODING_SESSION_TURNS_PATH, DOCS_API_PATH, DOCS_INDEX_PATH, DOCS_ROOT_PATH, DOCS_STYLES_PATH,
    LIVENESS_PATH, OPENAPI_PATH, ProblemDocument, REVISIONS_PATH, TASK_APPROVAL_PATH,
    TASK_APPROVALS_PATH, TASK_EVENTS_PATH, TASK_PATH, TASKS_PATH, TRIGGERS_PATH,
};
use agent_platform_app::{
    Application, ApplicationError, ApprovalContinuation, TaskExecutionPlan, TrustedRequestContext,
};
use agent_platform_auth::{
    AttemptConnectorAccess, AttemptWorkspaceAccess, CredentialVerifier, UserModelLease, operation,
};
use agent_platform_core::{
    ActivateRevision, AgentId, ApprovalId, AttemptId, CapabilityProfileId, ConnectorOwnerContext,
    ConversationInput, CreateAgent, CreateCapabilityProfile, CreateTrigger, PendingApproval,
    RequestId, ResolveApproval, RevisionSpec, SubmitTask, TaskEventKind, TaskFailure, TaskId,
    TenantId, UpdateCapabilityProfile,
};
use agent_platform_harness::{
    ApprovalDecision, ApprovalPort, ConnectorApprovalEvidence, ExecutionError, LoopEvent,
    UserModelExecution, UserModelRunOutcome, UserModelRunner,
};
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, Extension, Path, State};
use axum::http::{HeaderMap, Request, StatusCode, header};
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
}

impl HttpState {
    pub fn new(app: Application, verifier: Arc<dyn CredentialVerifier>) -> Self {
        Self {
            app,
            verifier,
            runner: None,
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
    workspace_access: Option<Arc<AttemptWorkspaceAccess>>,
}

#[derive(Debug, Clone)]
struct ApprovalTarget {
    operation: String,
    connection: String,
    description: String,
}

#[derive(Clone, Default)]
struct DeferredApprovalCapture(Arc<Mutex<Option<PendingApproval>>>);

impl DeferredApprovalCapture {
    fn publish(&self, approval: PendingApproval) -> Result<(), ()> {
        let mut pending = self.0.lock().map_err(|_| ())?;
        if pending.is_some() {
            return Err(());
        }
        *pending = Some(approval);
        Ok(())
    }

    fn take(&self) -> Result<PendingApproval, ()> {
        self.0.lock().map_err(|_| ())?.take().ok_or(())
    }
}

struct AttemptApprover {
    task_id: TaskId,
    attempt_id: AttemptId,
    context: ConnectorOwnerContext,
    targets: BTreeMap<String, ApprovalTarget>,
    deferred: DeferredApprovalCapture,
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
        if self.deferred.publish(pending).is_err() {
            return ApprovalDecision::denied("the approval checkpoint handoff is unavailable");
        }
        ApprovalDecision::deferred(approval_id.to_string())
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
        .route(CODING_SESSION_TURNS_PATH, post(submit_coding_session_turn))
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
        && matches!(request.uri().path(), TASKS_PATH | CODING_SESSION_TURNS_PATH))
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
    let (authority, lease, connector_access, workspace_access) = authenticated.into_parts();
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
            workspace_access,
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
    admit_submitted_task(&state, &context, attempt, request)
}

async fn submit_coding_session_turn(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Extension(attempt): Extension<AttemptAdmission>,
    Json(request): Json<SubmitTask>,
) -> Response {
    if !matches!(
        serde_json::from_value::<ConversationInput>(request.input.clone()),
        Ok(ConversationInput::CodingSessionTurn { .. })
    ) {
        return problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            "coding_session_turn_invalid",
            "the coding endpoint requires a coding_session_turn input",
        );
    }
    admit_submitted_task(&state, &context, attempt, request)
}

fn admit_submitted_task(
    state: &HttpState,
    context: &TrustedRequestContext,
    attempt: AttemptAdmission,
    request: SubmitTask,
) -> Response {
    let admission = match state
        .app
        .admit_task(context, request, attempt.attempt_id.clone())
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
            attempt.workspace_access,
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
    result(
        StatusCode::OK,
        state.app.list_task_approvals(&context, &task_id),
    )
}

async fn resolve_task_approval(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    headers: HeaderMap,
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
    let approval = match state.app.resolve_task_approval(
        &context,
        &task_id,
        &approval_id,
        now_ms(),
        &resolution,
    ) {
        Ok(approval) => approval,
        Err(error) => return application_error(&error),
    };
    let Some(runner) = state.runner.clone() else {
        return problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "execution_unavailable",
            "hosted execution is not configured",
        );
    };
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let authenticated = match state
        .verifier
        .verify(authorization, Some(&task.attempt_id))
        .await
    {
        Ok(authenticated) if authenticated.authority() == context.authority() => authenticated,
        Ok(_) => {
            return problem(
                StatusCode::UNAUTHORIZED,
                "continuation_authority_changed",
                "the continuation authority differs from the approval authority",
            );
        }
        Err(error) => {
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "continuation_authority_unavailable",
                error.reason(),
            );
        }
    };
    let (_, lease, connector_access, workspace_access) = authenticated.into_parts();
    let Some(lease) = lease else {
        return problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "model_lease_unavailable",
            "the approved attempt could not reacquire its user-bound model lease",
        );
    };
    let continuation = match state
        .app
        .claim_task_approval(&context, &task_id, &approval_id)
    {
        Ok(Some(continuation)) => continuation,
        Ok(None) => return (StatusCode::ACCEPTED, Json(approval)).into_response(),
        Err(error) => return application_error(&error),
    };
    spawn_approval_resume(
        state.app.clone(),
        runner,
        task,
        continuation,
        lease,
        connector_access,
        workspace_access,
    );
    (StatusCode::ACCEPTED, Json(approval)).into_response()
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
    workspace_access: Option<Arc<AttemptWorkspaceAccess>>,
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
        let emit = task_event_sink(&app, &tenant_id, &task_id, &attempt_id);
        let approval_evidence = ConnectorApprovalEvidence::default();
        let approval_context = ConnectorOwnerContext {
            tenant_id: tenant_id.clone(),
            agent_id: plan.task.agent_id.clone(),
            agent_revision: plan.task.agent_revision,
            authority_snapshot_id: plan.task.request_id.clone(),
            authority_snapshot_sha256: authority_snapshot_sha256(&plan),
        };
        let deferred = DeferredApprovalCapture::default();
        let approver = AttemptApprover {
            task_id: task_id.clone(),
            attempt_id: attempt_id.clone(),
            context: approval_context.clone(),
            targets: approval_targets(&plan),
            deferred: deferred.clone(),
        };
        let result = runner
            .execute(
                UserModelExecution {
                    task_id: task_id.clone(),
                    revision: plan.revision.clone(),
                    toolset: plan.toolset.clone(),
                    input: plan.task.input.clone(),
                    lease,
                    connector_access,
                    workspace_access,
                    attempt_id: attempt_id.clone(),
                    connector_context: connector_operation_context(&approval_context),
                    approval_evidence,
                    approvals: Box::new(approver),
                },
                emit,
            )
            .await;
        settle_execution(&app, &plan, &deferred, result);
    });
}

fn spawn_approval_resume(
    app: Application,
    runner: UserModelRunner,
    task: agent_platform_core::Task,
    continuation: ApprovalContinuation,
    lease: Arc<UserModelLease>,
    connector_access: Option<Arc<AttemptConnectorAccess>>,
    workspace_access: Option<Arc<AttemptWorkspaceAccess>>,
) {
    tokio::spawn(async move {
        let plan = TaskExecutionPlan {
            task,
            revision: continuation.revision,
            toolset: continuation.toolset,
        };
        let tenant_id = plan.task.tenant_id.clone();
        let task_id = plan.task.id.clone();
        let attempt_id = plan.task.attempt_id.clone();
        let Ok(checkpoint) = serde_json::from_value(continuation.checkpoint) else {
            fail_execution(
                &app,
                &plan,
                "approval_checkpoint_invalid",
                "the persisted Harness approval checkpoint is invalid",
            );
            return;
        };
        let evidence = ConnectorApprovalEvidence::default();
        let decision = match continuation.resolution {
            ResolveApproval::Approve {
                approval_evidence_ref,
            } => {
                if evidence
                    .approve(continuation.approval.call_id.clone(), approval_evidence_ref)
                    .is_err()
                {
                    fail_execution(
                        &app,
                        &plan,
                        "approval_evidence_unavailable",
                        "the exact Connector approval evidence could not be restored",
                    );
                    return;
                }
                ApprovalDecision::Approved
            }
            ResolveApproval::Deny { reason } => ApprovalDecision::denied(reason),
        };
        let context = continuation.approval.context;
        let deferred = DeferredApprovalCapture::default();
        let approver = AttemptApprover {
            task_id: task_id.clone(),
            attempt_id: attempt_id.clone(),
            context: context.clone(),
            targets: approval_targets(&plan),
            deferred: deferred.clone(),
        };
        let emit = task_event_sink(&app, &tenant_id, &task_id, &attempt_id);
        let result = runner
            .resume_approval(
                UserModelExecution {
                    task_id: task_id.clone(),
                    revision: plan.revision.clone(),
                    toolset: plan.toolset.clone(),
                    input: plan.task.input.clone(),
                    lease,
                    connector_access,
                    workspace_access,
                    attempt_id,
                    connector_context: connector_operation_context(&context),
                    approval_evidence: evidence,
                    approvals: Box::new(approver),
                },
                checkpoint,
                decision,
                emit,
            )
            .await;
        settle_execution(&app, &plan, &deferred, result);
    });
}

fn settle_execution(
    app: &Application,
    plan: &TaskExecutionPlan,
    deferred: &DeferredApprovalCapture,
    result: Result<UserModelRunOutcome, ExecutionError>,
) {
    match result {
        Ok(UserModelRunOutcome::Completed { output }) => {
            let _ = app.succeed_task(
                &plan.task.tenant_id,
                &plan.task.id,
                &plan.task.attempt_id,
                now_ms(),
                output,
            );
        }
        Ok(UserModelRunOutcome::AwaitingApproval { checkpoint }) => {
            let suspended = deferred.take().and_then(|approval| {
                serde_json::to_value(checkpoint)
                    .map(|checkpoint| (approval, checkpoint))
                    .map_err(|_| ())
            });
            let Ok((approval, checkpoint)) = suspended else {
                fail_execution(
                    app,
                    plan,
                    "approval_checkpoint_unavailable",
                    "Harness suspended without a complete approval checkpoint",
                );
                return;
            };
            if app
                .suspend_task_for_approval(&plan.task.tenant_id, plan, &approval, checkpoint)
                .is_err()
            {
                fail_execution(
                    app,
                    plan,
                    "approval_checkpoint_persistence_failed",
                    "the approval checkpoint could not be persisted",
                );
            }
        }
        Err(error) => fail_execution(app, plan, error.code(), &error.to_string()),
    }
}

fn fail_execution(app: &Application, plan: &TaskExecutionPlan, code: &str, message: &str) {
    let _ = app.fail_task(
        &plan.task.tenant_id,
        &plan.task.id,
        &plan.task.attempt_id,
        now_ms(),
        TaskFailure {
            code: code.to_owned(),
            message: message.to_owned(),
        },
    );
}

fn task_event_sink(
    app: &Application,
    tenant_id: &TenantId,
    task_id: &TaskId,
    attempt_id: &AttemptId,
) -> Arc<dyn Fn(LoopEvent) + Send + Sync> {
    let app = app.clone();
    let tenant_id = tenant_id.clone();
    let task_id = task_id.clone();
    let attempt_id = attempt_id.clone();
    Arc::new(move |event: LoopEvent| match event {
        LoopEvent::TextDelta { text } => {
            let _ = app.append_task_text(&tenant_id, &task_id, &attempt_id, now_ms(), text);
        }
        LoopEvent::ContextChanged { revision, .. } => {
            let _ = app.append_task_context_changed(
                &tenant_id,
                &task_id,
                &attempt_id,
                now_ms(),
                revision,
            );
        }
        LoopEvent::InventoryChanged {
            revision,
            published_tools,
        } => {
            let _ = app.append_task_inventory_changed(
                &tenant_id,
                &task_id,
                &attempt_id,
                now_ms(),
                revision,
                published_tools
                    .into_iter()
                    .map(|name| name.to_string())
                    .collect(),
            );
        }
        _ => {}
    })
}

fn approval_targets(plan: &TaskExecutionPlan) -> BTreeMap<String, ApprovalTarget> {
    plan.toolset.as_ref().map_or_else(BTreeMap::new, |toolset| {
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
    })
}

fn connector_operation_context(context: &ConnectorOwnerContext) -> operation::OwnerContext {
    operation::OwnerContext {
        tenant_id: context.tenant_id.to_string(),
        agent_id: context.agent_id.to_string(),
        agent_revision: context.agent_revision,
        authority_snapshot_id: context.authority_snapshot_id.to_string(),
        authority_snapshot_sha256: context.authority_snapshot_sha256.clone(),
    }
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
        | ApplicationError::TaskNotFound
        | ApplicationError::ApprovalNotFound => (StatusCode::NOT_FOUND, "not_found"),
        ApplicationError::ActiveRevisionConflict { .. }
        | ApplicationError::CapabilityProfileRevisionConflict { .. }
        | ApplicationError::IdempotencyConflict
        | ApplicationError::ApprovalConflict => (StatusCode::CONFLICT, "conflict"),
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

    fn service_for(app: Application, token: &str, subject: &str) -> Router {
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
            SubjectId::new(subject).unwrap(),
            None,
            None,
            scopes,
        )
        .unwrap();
        let verifier = DevelopmentVerifier::new(token, authority).unwrap();
        router(HttpState::new(app, Arc::new(verifier)))
    }

    fn service() -> Router {
        service_for(
            Application::new(Arc::new(EmptyCatalog)),
            "a-development-secret",
            "human-alice",
        )
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
    async fn coding_endpoint_refuses_an_untyped_task_input() {
        let response = service()
            .oneshot(request(
                Method::POST,
                CODING_SESSION_TURNS_PATH,
                r#"{"agent_id":"agent-one","idempotency_key":"turn-one","input":{"prompt":"hello"}}"#,
                Some("a-development-secret"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            json_body(response).await["code"],
            "coding_session_turn_invalid"
        );
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

    #[tokio::test]
    async fn same_tenant_http_callers_cannot_enumerate_or_fetch_foreign_agents() {
        let application = Application::new(Arc::new(EmptyCatalog));
        let alice = service_for(
            application.clone(),
            "alice-development-secret",
            "human-alice",
        );
        let bob = service_for(application, "bob-development-secret", "human-bob");
        let created = alice
            .clone()
            .oneshot(request(
                Method::POST,
                "/v1/agents",
                r#"{"name":"Alice private agent"}"#,
                Some("alice-development-secret"),
            ))
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let created = json_body(created).await;
        let agent_id = created["id"].as_str().unwrap();

        let alice_agents = alice
            .oneshot(request(
                Method::GET,
                "/v1/agents",
                "",
                Some("alice-development-secret"),
            ))
            .await
            .unwrap();
        assert_eq!(alice_agents.status(), StatusCode::OK);
        assert_eq!(json_body(alice_agents).await.as_array().unwrap().len(), 1);

        let bob_agents = bob
            .clone()
            .oneshot(request(
                Method::GET,
                "/v1/agents",
                "",
                Some("bob-development-secret"),
            ))
            .await
            .unwrap();
        assert_eq!(bob_agents.status(), StatusCode::OK);
        assert!(json_body(bob_agents).await.as_array().unwrap().is_empty());

        let foreign = bob
            .oneshot(request(
                Method::GET,
                &format!("/v1/agents/{agent_id}"),
                "",
                Some("bob-development-secret"),
            ))
            .await
            .unwrap();
        assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
        assert_eq!(json_body(foreign).await["code"], "not_found");
    }

    #[test]
    fn an_attempt_approver_defers_without_blocking_and_captures_the_exact_call() {
        let deferred = DeferredApprovalCapture::default();
        let context = ConnectorOwnerContext {
            tenant_id: TenantId::new("tenant-one").unwrap(),
            agent_id: AgentId::new("agent-one").unwrap(),
            agent_revision: 1,
            authority_snapshot_id: RequestId::new("request-one").unwrap(),
            authority_snapshot_sha256: "a".repeat(64),
        };
        let mut approver = AttemptApprover {
            task_id: TaskId::new("task-one").unwrap(),
            attempt_id: AttemptId::new("attempt-one").unwrap(),
            context: context.clone(),
            targets: BTreeMap::from([(
                "todo_create_item".to_owned(),
                ApprovalTarget {
                    operation: "todo.item.create".to_owned(),
                    connection: "todo".to_owned(),
                    description: "description-one".to_owned(),
                },
            )]),
            deferred: deferred.clone(),
        };
        let spec = harness_wire::ToolSpec {
            name: harness_wire::ToolName::new("todo_create_item").unwrap(),
            description: "create a todo".to_owned(),
            input_schema: json!({"type": "object"}),
            approval: harness_wire::Approval::Required,
            envelope: harness_wire::Envelope::default(),
        };
        let call = harness_wire::ToolCall {
            call_id: harness_wire::CallId::new("call-one").unwrap(),
            name: spec.name.clone(),
            arguments: json!({"list_id": "list-one", "title": "Ship it"}),
        };

        let decision = approver.decide(&call, &spec);
        assert!(matches!(decision, ApprovalDecision::Deferred { .. }));
        let approval = deferred.take().unwrap();
        assert_eq!(approval.task_id.as_str(), "task-one");
        assert_eq!(approval.attempt_id.as_str(), "attempt-one");
        assert_eq!(approval.call_id, "call-one");
        assert_eq!(approval.operation_ref, "todo.item.create");
        assert_eq!(approval.input, call.arguments);
        assert_eq!(approval.context, context);
    }
}
