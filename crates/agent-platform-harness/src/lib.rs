#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use agent_platform_auth::{
    AttemptConnectorAccess, AttemptWorkspaceAccess, UserModelLease, operation,
};
use agent_platform_connectors::{CompiledCapability, CompiledToolset};
use agent_platform_core::{AttemptId, ConversationInput, ConversationRole, RevisionSpec, TaskId};
use agentide_contracts::{ActorView, ContextPack, SelectionKind};
use agentide_harness::inventory_specs;
use harness_loop::{
    AgentLoop, ApprovalCheckpoint, ContextCacheClass, ContextKind, ContextLayer, ContextPackage,
    ContextTrust, EnvironmentError, LoopConfig, LoopError, LoopOutcome, LoopSink, LoopStop,
    TurnEnvironment, TurnEnvironmentProvider, TurnEnvironmentRequest,
};
use harness_messages::{Endpoint, MessagesClient};
use harness_wire::{
    Bearer, BearerSource, CredentialKind, ModelPort, Subject, ToolCall, ToolOutcome, ToolPort,
    ToolSpec, WireError, WireErrorCode,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use workspace_core::{CodingActorViewRequest, CodingIntentInvocation};

pub use harness_loop::{ApprovalDecision, ApprovalPort, LoopEvent};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorInvocation {
    pub call_id: String,
    pub operation_ref: String,
    pub connection_ref: String,
    pub description_ref: String,
    pub input: Value,
    pub approval_evidence_ref: Option<String>,
}

pub trait ConnectorInvoker: Send + Sync {
    fn invoke(&self, request: ConnectorInvocation) -> ToolOutcome;
}

#[derive(Clone)]
pub struct ConnectorTools {
    specs: Vec<ToolSpec>,
    capabilities: BTreeMap<String, CompiledCapability>,
    invoker: Arc<dyn ConnectorInvoker>,
}

impl std::fmt::Debug for ConnectorTools {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConnectorTools")
            .field("specs", &self.specs)
            .field("capabilities", &self.capabilities)
            .field("invoker", &"ConnectorInvoker")
            .finish()
    }
}

impl ConnectorTools {
    pub fn new(toolset: &CompiledToolset, invoker: Arc<dyn ConnectorInvoker>) -> Self {
        let specs = toolset
            .capabilities
            .iter()
            .map(|capability| capability.tool.clone())
            .collect();
        let capabilities = toolset
            .capabilities
            .iter()
            .cloned()
            .map(|capability| (capability.tool.name.to_string(), capability))
            .collect();
        Self {
            specs,
            capabilities,
            invoker,
        }
    }
}

impl ToolPort for ConnectorTools {
    fn specs(&self) -> &[ToolSpec] {
        &self.specs
    }

    fn subjects(&self, call: &ToolCall) -> Vec<Subject> {
        self.capabilities
            .get(call.name.as_str())
            .map_or_else(Vec::new, |capability| {
                vec![Subject::host(format!(
                    "connectors/{}",
                    capability.connection_ref
                ))]
            })
    }

    fn call(&mut self, call: &ToolCall) -> ToolOutcome {
        let Some(capability) = self.capabilities.get(call.name.as_str()) else {
            return ToolOutcome::failed(format!(
                "tool `{}` is absent from the compiled capability profile",
                call.name
            ));
        };
        self.invoker.invoke(ConnectorInvocation {
            call_id: call.call_id.as_str().to_owned(),
            operation_ref: capability.operation_ref.clone(),
            connection_ref: capability.connection_ref.clone(),
            description_ref: capability.description_ref.clone(),
            input: call.arguments.clone(),
            approval_evidence_ref: None,
        })
    }
}

#[derive(Clone)]
struct WorkspaceTools {
    specs: Vec<ToolSpec>,
    access: Arc<AttemptWorkspaceAccess>,
    task_id: TaskId,
    attempt_id: AttemptId,
    workspace_session_id: String,
    agentide_session_id: String,
    handle: tokio::runtime::Handle,
}

impl std::fmt::Debug for WorkspaceTools {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WorkspaceTools")
            .field("specs", &self.specs)
            .field("access", &self.access)
            .field("task_id", &self.task_id)
            .field("attempt_id", &self.attempt_id)
            .field("workspace_session_id", &self.workspace_session_id)
            .field("agentide_session_id", &self.agentide_session_id)
            .finish_non_exhaustive()
    }
}

impl ToolPort for WorkspaceTools {
    fn specs(&self) -> &[ToolSpec] {
        &self.specs
    }

    fn subjects(&self, call: &ToolCall) -> Vec<Subject> {
        call.arguments
            .get("path")
            .and_then(Value::as_str)
            .map_or_else(
                || vec![Subject::file(".")],
                |path| vec![Subject::file(path)],
            )
    }

    fn operation(&self, call: &ToolCall) -> Option<String> {
        Some(format!("agentide.intent/{}", call.name))
    }

    fn call(&mut self, call: &ToolCall) -> ToolOutcome {
        let request = CodingIntentInvocation {
            agentide_session_id: self.agentide_session_id.clone(),
            task_id: self.task_id.to_string(),
            attempt_id: self.attempt_id.to_string(),
            call_id: call.call_id.to_string(),
            intent: call.name.to_string(),
            arguments: call.arguments.clone(),
        };
        match self.handle.block_on(self.access.invoke(
            &self.attempt_id,
            &self.workspace_session_id,
            &request,
        )) {
            Ok(result) => ToolOutcome::ok(result.output),
            Err(error) => ToolOutcome::failed(error.reason()),
        }
    }
}

enum ExecutionTools {
    Connectors(ConnectorTools),
    Workspace(WorkspaceTools),
}

impl ToolPort for ExecutionTools {
    fn specs(&self) -> &[ToolSpec] {
        match self {
            Self::Connectors(tools) => tools.specs(),
            Self::Workspace(tools) => tools.specs(),
        }
    }

    fn subjects(&self, call: &ToolCall) -> Vec<Subject> {
        match self {
            Self::Connectors(tools) => tools.subjects(call),
            Self::Workspace(tools) => tools.subjects(call),
        }
    }

    fn operation(&self, call: &ToolCall) -> Option<String> {
        match self {
            Self::Connectors(tools) => tools.operation(call),
            Self::Workspace(tools) => tools.operation(call),
        }
    }

    fn call(&mut self, call: &ToolCall) -> ToolOutcome {
        match self {
            Self::Connectors(tools) => tools.call(call),
            Self::Workspace(tools) => tools.call(call),
        }
    }
}

#[derive(Clone)]
struct CodingSessionCoordinates {
    workspace_session_id: String,
    agentide_session_id: String,
}

struct WorkspaceSetup {
    coordinates: CodingSessionCoordinates,
    access: Arc<AttemptWorkspaceAccess>,
    initial: ActorView,
    specs: Vec<ToolSpec>,
}

fn coding_session_coordinates(input: &Value) -> Option<CodingSessionCoordinates> {
    let ConversationInput::CodingSessionTurn {
        workspace_session_id,
        agentide_session_id,
        ..
    } = serde_json::from_value(input.clone()).ok()?
    else {
        return None;
    };
    Some(CodingSessionCoordinates {
        workspace_session_id,
        agentide_session_id,
    })
}

async fn prepare_workspace(
    input: &Value,
    workspace_access: Option<Arc<AttemptWorkspaceAccess>>,
    task_id: &TaskId,
    attempt_id: &AttemptId,
    context_window: u64,
) -> Result<Option<WorkspaceSetup>, ExecutionError> {
    let Some(coordinates) = coding_session_coordinates(input) else {
        return Ok(None);
    };
    let access = workspace_access.ok_or(ExecutionError::WorkspaceUnavailable)?;
    let request = CodingActorViewRequest {
        agentide_session_id: coordinates.agentide_session_id.clone(),
        task_id: task_id.to_string(),
        attempt_id: attempt_id.to_string(),
        turn: 1,
    };
    let initial = access
        .actor_view(attempt_id, &coordinates.workspace_session_id, &request)
        .await
        .map_err(|_| ExecutionError::WorkspaceUnavailable)?;
    if initial.actor.attempt.as_deref() != Some(attempt_id.as_str()) {
        return Err(ExecutionError::WorkspaceUnavailable);
    }
    initial
        .context
        .validate_model_attachments(
            usize::try_from(context_window).map_err(|_| ExecutionError::RequestTooLarge)?,
        )
        .map_err(|_| ExecutionError::RequestTooLarge)?;
    let specs =
        inventory_specs(&initial.inventory).map_err(|_| ExecutionError::HarnessConfiguration)?;
    Ok(Some(WorkspaceSetup {
        coordinates,
        access,
        initial,
        specs,
    }))
}

struct WorkspaceEnvironment {
    access: Arc<AttemptWorkspaceAccess>,
    task_id: TaskId,
    attempt_id: AttemptId,
    coordinates: CodingSessionCoordinates,
    handle: tokio::runtime::Handle,
    initial: Option<ActorView>,
}

impl TurnEnvironmentProvider for WorkspaceEnvironment {
    fn refresh(
        &mut self,
        request: TurnEnvironmentRequest,
    ) -> Result<TurnEnvironment, EnvironmentError> {
        let view = if request.turn == 1 {
            self.initial
                .take()
                .map_or_else(|| self.fetch(request.turn), Ok)?
        } else {
            self.fetch(request.turn)?
        };
        if view.actor.attempt.as_deref() != Some(self.attempt_id.as_str()) {
            return Err(EnvironmentError::Invalid(
                "Workspace returned an actor view for another attempt".into(),
            ));
        }
        let context_window = request
            .context_window
            .and_then(|window| usize::try_from(window).ok())
            .ok_or_else(|| {
                EnvironmentError::Invalid("the model context window is unavailable".into())
            })?;
        view.context
            .validate_model_attachments(context_window)
            .map_err(EnvironmentError::Invalid)?;
        let tools = inventory_specs(&view.inventory)
            .map_err(|error| EnvironmentError::Invalid(error.to_string()))?;
        let context = context_package(&view.context)?;
        let context_revision = stable_context_revision(&view.context)?;
        Ok(TurnEnvironment {
            context,
            tools,
            context_revision,
            inventory_revision: format!("sha256:{}", view.inventory.digest),
        })
    }
}

fn stable_context_revision(context: &ContextPack) -> Result<String, EnvironmentError> {
    let mut stable_context = context.clone();
    // Workspace's monotonic revision proves that a refresh happened. It is not part of the
    // semantic context identity: unchanged content must remain cacheable across model turns.
    stable_context.revision = 0;
    let encoded = serde_json::to_vec(&stable_context)
        .map_err(|error| EnvironmentError::Invalid(error.to_string()))?;
    Ok(format!("sha256:{}", hex::encode(Sha256::digest(encoded))))
}

impl WorkspaceEnvironment {
    fn fetch(&self, turn: u64) -> Result<ActorView, EnvironmentError> {
        let request = CodingActorViewRequest {
            agentide_session_id: self.coordinates.agentide_session_id.clone(),
            task_id: self.task_id.to_string(),
            attempt_id: self.attempt_id.to_string(),
            turn,
        };
        self.handle
            .block_on(self.access.actor_view(
                &self.attempt_id,
                &self.coordinates.workspace_session_id,
                &request,
            ))
            .map_err(|error| EnvironmentError::Unavailable(error.reason().to_owned()))
    }
}

fn context_package(context: &ContextPack) -> Result<ContextPackage, EnvironmentError> {
    let mut metadata = context.clone();
    // Workspace's monotonic transport revision records the refresh. It must not make identical
    // actor context look changed to Harness on every model turn.
    metadata.revision = 0;
    metadata.pins.clear();
    metadata.focused_selections.clear();
    let body = serde_json::to_string_pretty(&metadata)
        .map_err(|error| EnvironmentError::Invalid(error.to_string()))?;
    let mut package = ContextPackage::new(vec![
        ContextLayer::new(
            "agentide-session-context",
            ContextKind::ProvidedContext,
            ContextTrust::Workspace,
            ContextCacheClass::Turn,
            body,
        )
        .with_source(format!("revision:{}", context.source_revision)),
    ]);
    for (index, selection) in context
        .focused_selections
        .iter()
        .chain(&context.pins)
        .enumerate()
    {
        let kind = match selection.kind {
            SelectionKind::Editor => "editor",
            SelectionKind::DiffHunk => "diff-hunk",
            SelectionKind::Terminal => "terminal-selection",
            SelectionKind::Process => "process-result",
            SelectionKind::Evidence => "evidence",
        };
        package.push(
            ContextLayer::new(
                format!("agentide-selection-{index}-{kind}"),
                ContextKind::ProvidedContext,
                ContextTrust::Workspace,
                ContextCacheClass::Turn,
                selection.content.clone(),
            )
            .with_source(selection.reference.clone()),
        );
    }
    Ok(package)
}

pub fn run(
    model: &mut dyn ModelPort,
    tools: &mut ConnectorTools,
    revision: &RevisionSpec,
    input: impl Into<String>,
    sink: &mut dyn LoopSink,
    approvals: &mut dyn ApprovalPort,
) -> Result<LoopOutcome, LoopError> {
    AgentLoop::new(
        model,
        tools,
        approvals,
        LoopConfig::new(revision.model.clone(), revision.instructions.clone()),
    )
    .run(input, sink)
}

#[derive(Debug, Clone)]
pub struct UserModelRunner {
    endpoint_base: String,
    context_window: u64,
}

pub struct UserModelExecution {
    pub task_id: TaskId,
    pub revision: RevisionSpec,
    pub toolset: Option<CompiledToolset>,
    pub input: Value,
    pub lease: Arc<UserModelLease>,
    pub connector_access: Option<Arc<AttemptConnectorAccess>>,
    pub workspace_access: Option<Arc<AttemptWorkspaceAccess>>,
    pub attempt_id: AttemptId,
    pub connector_context: operation::OwnerContext,
    pub approval_evidence: ConnectorApprovalEvidence,
    pub approvals: Box<dyn ApprovalPort + Send>,
}

#[derive(Debug)]
pub enum UserModelRunOutcome {
    Completed { output: String },
    AwaitingApproval { checkpoint: Box<ApprovalCheckpoint> },
}

impl UserModelRunner {
    pub fn new(
        endpoint_base: impl Into<String>,
        context_window: u64,
    ) -> Result<Self, ExecutionError> {
        let endpoint_base = endpoint_base.into();
        if context_window == 0 || endpoint_base.trim().is_empty() {
            return Err(ExecutionError::Configuration);
        }
        Ok(Self {
            endpoint_base,
            context_window,
        })
    }

    pub async fn execute(
        &self,
        execution: UserModelExecution,
        emit_event: Arc<dyn Fn(LoopEvent) + Send + Sync>,
    ) -> Result<UserModelRunOutcome, ExecutionError> {
        let prompt = task_prompt(&execution.input)?;
        self.drive(execution, Some(prompt), None, emit_event).await
    }

    pub async fn resume_approval(
        &self,
        execution: UserModelExecution,
        checkpoint: ApprovalCheckpoint,
        decision: ApprovalDecision,
        emit_event: Arc<dyn Fn(LoopEvent) + Send + Sync>,
    ) -> Result<UserModelRunOutcome, ExecutionError> {
        self.drive(execution, None, Some((checkpoint, decision)), emit_event)
            .await
    }

    async fn drive(
        &self,
        execution: UserModelExecution,
        prompt: Option<String>,
        continuation: Option<(ApprovalCheckpoint, ApprovalDecision)>,
        emit_event: Arc<dyn Fn(LoopEvent) + Send + Sync>,
    ) -> Result<UserModelRunOutcome, ExecutionError> {
        let UserModelExecution {
            task_id,
            revision,
            toolset,
            input,
            lease,
            connector_access,
            workspace_access,
            attempt_id,
            connector_context,
            approval_evidence,
            mut approvals,
        } = execution;
        let workspace_setup = prepare_workspace(
            &input,
            workspace_access,
            &task_id,
            &attempt_id,
            self.context_window,
        )
        .await?;
        let endpoint_base = self.endpoint_base.clone();
        let context_window = self.context_window;
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let endpoint = Endpoint::new(endpoint_base, revision.model.clone(), context_window)
                .map_err(|_| ExecutionError::Configuration)?;
            let source = Arc::new(LeaseBearerSource {
                lease,
                handle: handle.clone(),
            });
            let mut model = MessagesClient::new(endpoint, source)
                .map_err(|error| execution_wire_error(&error))?;
            let toolset = toolset.unwrap_or_else(empty_toolset);
            let invoker: Arc<dyn ConnectorInvoker> = connector_access.map_or_else(
                || Arc::new(RefusingInvoker) as Arc<dyn ConnectorInvoker>,
                |access| {
                    Arc::new(HostedConnectorInvoker {
                        access,
                        attempt_id: attempt_id.clone(),
                        context: connector_context,
                        approval_evidence,
                        handle: handle.clone(),
                    }) as Arc<dyn ConnectorInvoker>
                },
            );
            let mut environment = None;
            let mut tools = if let Some(WorkspaceSetup {
                coordinates,
                access,
                initial,
                specs,
            }) = workspace_setup
            {
                environment = Some(WorkspaceEnvironment {
                    access: access.clone(),
                    task_id: task_id.clone(),
                    attempt_id: attempt_id.clone(),
                    coordinates: coordinates.clone(),
                    handle: handle.clone(),
                    initial: Some(initial),
                });
                ExecutionTools::Workspace(WorkspaceTools {
                    specs,
                    access,
                    task_id,
                    attempt_id: attempt_id.clone(),
                    workspace_session_id: coordinates.workspace_session_id,
                    agentide_session_id: coordinates.agentide_session_id,
                    handle: handle.clone(),
                })
            } else {
                ExecutionTools::Connectors(ConnectorTools::new(&toolset, invoker))
            };
            let mut sink = EventSink { emit_event };
            let config = LoopConfig::new(revision.model.clone(), revision.instructions.clone())
                .with_context_window(Some(context_window));
            let mut agent_loop = AgentLoop::new(&mut model, &mut tools, approvals.as_mut(), config);
            if let Some(environment) = environment.as_mut() {
                agent_loop = agent_loop.with_environment(environment);
            }
            let outcome = match (prompt, continuation) {
                (Some(prompt), None) => agent_loop.run(prompt, &mut sink),
                (None, Some((checkpoint, decision))) => {
                    agent_loop.resume_approval(checkpoint, decision, &mut sink)
                }
                _ => return Err(ExecutionError::HarnessConfiguration),
            }
            .map_err(|error| execution_loop_error(&error))?;
            execution_outcome(outcome)
        })
        .await
        .map_err(|_| ExecutionError::WorkerUnavailable)?
    }
}

fn execution_outcome(outcome: LoopOutcome) -> Result<UserModelRunOutcome, ExecutionError> {
    match outcome.stop {
        LoopStop::Completed => Ok(UserModelRunOutcome::Completed {
            output: outcome.text,
        }),
        LoopStop::AwaitingApproval { .. } => outcome
            .checkpoint
            .map(|checkpoint| UserModelRunOutcome::AwaitingApproval {
                checkpoint: Box::new(checkpoint),
            })
            .ok_or(ExecutionError::HarnessConfiguration),
        _ => Err(ExecutionError::Incomplete),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExecutionError {
    #[error("task input must be a string or an object containing a non-empty `prompt` string")]
    InvalidInput,
    #[error("the model endpoint configuration is invalid")]
    Configuration,
    #[error("the user-bound model credential is missing, expired, or refused")]
    CredentialUnavailable,
    #[error("the model provider could not be reached")]
    ProviderUnavailable,
    #[error("the model provider rate limited the attempt")]
    ProviderRateLimited,
    #[error("the selected model route or request was refused by the provider")]
    ProviderRefused,
    #[error("the model provider returned an unsupported response")]
    ProviderProtocol,
    #[error("the model request exceeded a declared bound")]
    RequestTooLarge,
    #[error("the model request requires an unsupported feature")]
    Unsupported,
    #[error("the model attempt was cancelled")]
    Cancelled,
    #[error("the Harness run configuration is invalid")]
    HarnessConfiguration,
    #[error("the Harness run stopped before completing")]
    Incomplete,
    #[error("the current Workspace coding session or actor view is unavailable")]
    WorkspaceUnavailable,
    #[error("the execution worker is unavailable")]
    WorkerUnavailable,
}

impl ExecutionError {
    /// Stable redaction-safe failure code stored in Task evidence.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput => "task_input_invalid",
            Self::Configuration => "model_endpoint_invalid",
            Self::CredentialUnavailable => "model_credential_unavailable",
            Self::ProviderUnavailable => "model_provider_unavailable",
            Self::ProviderRateLimited => "model_provider_rate_limited",
            Self::ProviderRefused => "model_route_refused",
            Self::ProviderProtocol => "model_provider_protocol_invalid",
            Self::RequestTooLarge => "model_request_too_large",
            Self::Unsupported => "model_feature_unsupported",
            Self::Cancelled => "model_attempt_cancelled",
            Self::HarnessConfiguration => "harness_configuration_invalid",
            Self::Incomplete => "harness_incomplete",
            Self::WorkspaceUnavailable => "workspace_actor_view_unavailable",
            Self::WorkerUnavailable => "execution_worker_unavailable",
        }
    }
}

fn execution_loop_error(error: &LoopError) -> ExecutionError {
    match error {
        LoopError::Wire(error) => execution_wire_error(error),
        LoopError::Budget(_) => ExecutionError::RequestTooLarge,
        LoopError::Config(_) | LoopError::Environment(_) => ExecutionError::HarnessConfiguration,
    }
}

fn execution_wire_error(error: &WireError) -> ExecutionError {
    match error.code {
        WireErrorCode::Transport => ExecutionError::ProviderUnavailable,
        WireErrorCode::Protocol => ExecutionError::ProviderProtocol,
        WireErrorCode::Unauthorized => ExecutionError::CredentialUnavailable,
        WireErrorCode::RateLimited => ExecutionError::ProviderRateLimited,
        WireErrorCode::Refused => ExecutionError::ProviderRefused,
        WireErrorCode::TooLarge => ExecutionError::RequestTooLarge,
        WireErrorCode::Unsupported => ExecutionError::Unsupported,
        WireErrorCode::Cancelled => ExecutionError::Cancelled,
    }
}

struct LeaseBearerSource {
    lease: Arc<UserModelLease>,
    handle: tokio::runtime::Handle,
}

impl std::fmt::Debug for LeaseBearerSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LeaseBearerSource")
            .field("lease", &self.lease)
            .finish_non_exhaustive()
    }
}

impl BearerSource for LeaseBearerSource {
    fn bearer(&self) -> Result<Bearer, WireError> {
        let credential = self.handle.block_on(self.lease.redeem()).map_err(|_| {
            WireError::unauthorized("the attempt model lease could not be redeemed")
        })?;
        Ok(Bearer::new(
            credential.expose_at_provider_boundary().to_owned(),
        ))
    }

    fn kind(&self) -> CredentialKind {
        CredentialKind::Oauth
    }
}

struct EventSink {
    emit_event: Arc<dyn Fn(LoopEvent) + Send + Sync>,
}

impl LoopSink for EventSink {
    fn emit(&mut self, event: LoopEvent) {
        (self.emit_event)(event);
    }
}

#[derive(Debug)]
struct RefusingInvoker;

impl ConnectorInvoker for RefusingInvoker {
    fn invoke(&self, _: ConnectorInvocation) -> ToolOutcome {
        ToolOutcome::failed("Connector invocation is not configured for this worker")
    }
}

/// Attempt-local handoff from the blocking human approval port to the following invocation.
#[derive(Clone, Debug, Default)]
pub struct ConnectorApprovalEvidence {
    values: Arc<Mutex<BTreeMap<String, String>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ApprovalEvidenceError {
    #[error("approval evidence already exists for the call")]
    AlreadyPresent,
    #[error("approval evidence storage is unavailable")]
    Unavailable,
}

impl ConnectorApprovalEvidence {
    pub fn approve(
        &self,
        call_id: String,
        approval_evidence_ref: String,
    ) -> Result<(), ApprovalEvidenceError> {
        let mut values = self
            .values
            .lock()
            .map_err(|_| ApprovalEvidenceError::Unavailable)?;
        if values.contains_key(&call_id) {
            return Err(ApprovalEvidenceError::AlreadyPresent);
        }
        values.insert(call_id, approval_evidence_ref);
        Ok(())
    }

    fn take(&self, call_id: &str) -> Option<String> {
        self.values.lock().ok()?.remove(call_id)
    }
}

struct HostedConnectorInvoker {
    access: Arc<AttemptConnectorAccess>,
    attempt_id: AttemptId,
    context: operation::OwnerContext,
    approval_evidence: ConnectorApprovalEvidence,
    handle: tokio::runtime::Handle,
}

impl ConnectorInvoker for HostedConnectorInvoker {
    fn invoke(&self, mut request: ConnectorInvocation) -> ToolOutcome {
        request.approval_evidence_ref = self.approval_evidence.take(&request.call_id);
        let operation = operation::InvokeRequest {
            operation_ref: request.operation_ref,
            connection_ref: request.connection_ref,
            description_ref: request.description_ref,
            input: request.input,
            approval_evidence_ref: request.approval_evidence_ref,
        };
        match self.handle.block_on(
            self.access
                .invoke(&self.attempt_id, &self.context, operation),
        ) {
            Ok(operation::OperationResult::Invoke(result)) => ToolOutcome::ok(result.output),
            Ok(_) => ToolOutcome::failed("Connector returned a non-invocation result"),
            Err(error) => ToolOutcome::failed(error.reason()),
        }
    }
}

fn empty_toolset() -> CompiledToolset {
    CompiledToolset {
        connector_contract: agent_platform_connectors::CONNECTOR_OPERATION_CONTRACT.to_owned(),
        digest_sha256: "0".repeat(64),
        capabilities: Vec::new(),
    }
}

fn task_prompt(input: &Value) -> Result<String, ExecutionError> {
    if let Ok(ConversationInput::ProjectConversation {
        prompt,
        messages,
        context,
    }) = serde_json::from_value::<ConversationInput>(input.clone())
    {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err(ExecutionError::InvalidInput);
        }
        let mut assembled = format!(
            "Read-only repository project context. Never claim to have observed files outside this exact snapshot.\nProject: {}\nProvider: {}\nProvider project: {}\nBranch: {}\nCommit: {}\n",
            context.path_with_namespace,
            context.provider,
            context.provider_project_ref,
            context.branch,
            context.commit
        );
        for file in context.files {
            assembled.push_str("\n--- file: ");
            assembled.push_str(&file.path);
            if file.truncated {
                assembled.push_str(" (truncated)");
            }
            assembled.push_str(" ---\n");
            assembled.push_str(&file.content);
            assembled.push('\n');
        }
        if !messages.is_empty() {
            assembled.push_str("\nConversation so far:\n");
        }
        for message in messages {
            let role = match message.role {
                ConversationRole::User => "user",
                ConversationRole::Assistant => "assistant",
                ConversationRole::System => "system",
            };
            assembled.push_str(role);
            assembled.push_str(": ");
            assembled.push_str(message.content.trim());
            assembled.push('\n');
        }
        assembled.push_str("\nCurrent user request:\n");
        assembled.push_str(prompt);
        return Ok(assembled);
    }
    if let Ok(ConversationInput::CodingSessionTurn {
        prompt, messages, ..
    }) = serde_json::from_value::<ConversationInput>(input.clone())
    {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            return Err(ExecutionError::InvalidInput);
        }
        let mut assembled = String::new();
        if !messages.is_empty() {
            assembled.push_str("Conversation so far:\n");
        }
        for message in messages {
            let role = match message.role {
                ConversationRole::User => "user",
                ConversationRole::Assistant => "assistant",
                ConversationRole::System => "system",
            };
            assembled.push_str(role);
            assembled.push_str(": ");
            assembled.push_str(message.content.trim());
            assembled.push('\n');
        }
        if !assembled.is_empty() {
            assembled.push('\n');
        }
        assembled.push_str("Current user request:\n");
        assembled.push_str(prompt);
        return Ok(assembled);
    }
    let prompt = match input {
        Value::String(prompt) => Some(prompt.as_str()),
        Value::Object(object) => object.get("prompt").and_then(Value::as_str),
        _ => None,
    }
    .map(str::trim)
    .filter(|prompt| !prompt.is_empty())
    .ok_or(ExecutionError::InvalidInput)?;
    Ok(prompt.to_owned())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use agent_platform_connectors::{CONNECTOR_OPERATION_CONTRACT, CompiledCapability};
    use agentide_contracts::ContextSelection;
    use harness_loop::{LoopStop, VecLoopSink};
    use harness_wire::{
        AccessKind, Approval, CallId, Effect, Envelope, Idempotency, Item, Risk, StopReason,
        StreamEvent, StreamSink, ToolName, TurnOutcome, TurnRequest, WireError, WireId,
    };
    use serde_json::json;

    use super::*;

    #[test]
    fn project_conversation_is_bound_to_the_exact_snapshot() {
        let prompt = task_prompt(&json!({
            "kind": "project_conversation",
            "prompt": "Where is startup configured?",
            "messages": [{"role":"user","content":"Inspect the service."}],
            "context": {
                "project_id": "project-one",
                "provider": "gitlab",
                "provider_project_ref": "42",
                "path_with_namespace": "group/service",
                "branch": "main",
                "commit": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "files": [{"path":"Cargo.toml","content":"[workspace]","truncated":false}]
            }
        }))
        .unwrap();
        assert!(prompt.contains("Project: group/service"));
        assert!(prompt.contains("Commit: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
        assert!(prompt.contains("--- file: Cargo.toml ---"));
        assert!(prompt.ends_with("Where is startup configured?"));
    }

    #[test]
    fn coding_conversation_preserves_history_without_claiming_ambient_files() {
        let prompt = task_prompt(&json!({
            "kind": "coding_session_turn",
            "prompt": "Apply the selected change.",
            "messages": [
                {"role":"user","content":"Inspect the parser."},
                {"role":"assistant","content":"The parser is in src/parser.rs."}
            ],
            "workspace_session_id": "workspace-one",
            "agentide_session_id": "agentide-one",
            "focused_selections": [],
            "open_files": [],
            "active_diff": null
        }))
        .unwrap();
        assert!(prompt.contains("user: Inspect the parser."));
        assert!(prompt.contains("assistant: The parser is in src/parser.rs."));
        assert!(prompt.ends_with("Current user request:\nApply the selected change."));
        assert!(!prompt.contains("Read-only repository project context"));
    }

    fn context_with_selection(revision: u64) -> ContextPack {
        let content = "fn selected() {}".to_owned();
        ContextPack {
            format: "agentide.context-pack/1".to_owned(),
            objective: "Repair the parser".to_owned(),
            source_revision: "a".repeat(40),
            focused_selections: vec![ContextSelection {
                id: "selection-one".to_owned(),
                kind: SelectionKind::Editor,
                reference: "src/parser.rs".to_owned(),
                start_line: Some(4),
                end_line: Some(4),
                sha256: hex::encode(Sha256::digest(content.as_bytes())),
                content,
                truncated: false,
            }],
            revision,
            ..ContextPack::default()
        }
    }

    #[test]
    fn actor_context_is_workspace_trusted_and_selection_bytes_are_isolated() {
        let package = context_package(&context_with_selection(7)).unwrap();
        assert_eq!(package.layers().len(), 2);
        assert_eq!(package.layers()[0].trust, ContextTrust::Workspace);
        assert_eq!(package.layers()[1].trust, ContextTrust::Workspace);
        assert_eq!(package.layers()[1].source.as_deref(), Some("src/parser.rs"));
        assert_eq!(package.layers()[1].body, "fn selected() {}");
        assert!(!package.layers()[0].body.contains("fn selected() {}"));
    }

    #[test]
    fn transport_revision_does_not_create_a_false_context_change() {
        let first = stable_context_revision(&context_with_selection(1)).unwrap();
        let second = stable_context_revision(&context_with_selection(2)).unwrap();
        assert_eq!(first, second);

        let mut changed = context_with_selection(3);
        changed.objective = "Repair and test the parser".to_owned();
        assert_ne!(first, stable_context_revision(&changed).unwrap());
    }

    #[derive(Debug, Default)]
    struct RecordingInvoker {
        requests: Mutex<Vec<ConnectorInvocation>>,
    }

    impl ConnectorInvoker for RecordingInvoker {
        fn invoke(&self, request: ConnectorInvocation) -> ToolOutcome {
            self.requests.lock().unwrap().push(request);
            ToolOutcome::ok(json!({"tickets": []}))
        }
    }

    struct ScriptedModel {
        wire: WireId,
        turns: u8,
    }

    impl ScriptedModel {
        fn new() -> Self {
            Self {
                wire: WireId::new("synthetic-wire").unwrap(),
                turns: 0,
            }
        }
    }

    impl ModelPort for ScriptedModel {
        fn wire(&self) -> &WireId {
            &self.wire
        }

        fn turn(
            &mut self,
            request: &TurnRequest,
            sink: &mut dyn StreamSink,
        ) -> Result<TurnOutcome, WireError> {
            assert_eq!(request.model, "model-one");
            assert_eq!(request.instructions, "Help with support.");
            assert_eq!(request.tools.len(), 1);
            self.turns += 1;
            if self.turns == 1 {
                return Ok(TurnOutcome {
                    stop_reason: StopReason::ToolCalls,
                    items: vec![Item::ToolCall(ToolCall {
                        call_id: CallId::new("call-one").unwrap(),
                        name: ToolName::new("list_support_tickets").unwrap(),
                        arguments: json!({"status": "open"}),
                    })],
                    usage: None,
                });
            }
            assert!(matches!(
                request.items.last(),
                Some(Item::ToolResult { .. })
            ));
            sink.emit(StreamEvent::TextDelta {
                text: "No open tickets.".to_owned(),
            });
            Ok(TurnOutcome {
                stop_reason: StopReason::EndTurn,
                items: vec![Item::assistant("No open tickets.")],
                usage: None,
            })
        }
    }

    fn toolset() -> CompiledToolset {
        CompiledToolset {
            connector_contract: CONNECTOR_OPERATION_CONTRACT.to_owned(),
            digest_sha256: "0".repeat(64),
            capabilities: vec![CompiledCapability {
                operation_ref: "support.ticket.list".to_owned(),
                connection_ref: "synthetic-support".to_owned(),
                description_ref: "description-one".to_owned(),
                tool: ToolSpec {
                    name: ToolName::new("list_support_tickets").unwrap(),
                    description: "List support tickets.".to_owned(),
                    input_schema: json!({"type": "object"}),
                    approval: Approval::NotRequired,
                    envelope: Envelope {
                        effects: vec![Effect::Read],
                        risk: Risk::Low,
                        idempotency: Idempotency::Idempotent,
                        access: vec![AccessKind::Network],
                    },
                },
            }],
        }
    }

    #[test]
    fn embedded_harness_runs_the_compiled_connector_tool_round_trip() {
        let invoker = Arc::new(RecordingInvoker::default());
        let mut tools = ConnectorTools::new(&toolset(), invoker.clone());
        let mut model = ScriptedModel::new();
        let revision = RevisionSpec {
            instructions: "Help with support.".to_owned(),
            model: "model-one".to_owned(),
            capability_profile_id: None,
            metadata: None,
        };
        let mut sink = VecLoopSink::new();
        let mut approvals = harness_loop::DenyAll;
        let outcome = run(
            &mut model,
            &mut tools,
            &revision,
            "List open support work.",
            &mut sink,
            &mut approvals,
        )
        .unwrap();
        assert_eq!(outcome.stop, LoopStop::Completed);
        assert_eq!(outcome.text, "No open tickets.");
        assert_eq!(sink.text(), "No open tickets.");
        let requests = invoker.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].operation_ref, "support.ticket.list");
        assert_eq!(requests[0].connection_ref, "synthetic-support");
        assert_eq!(requests[0].input, json!({"status": "open"}));
    }

    #[test]
    fn approval_evidence_is_exact_call_and_single_use() {
        let evidence = ConnectorApprovalEvidence::default();
        evidence
            .approve("call-one".to_owned(), "proof-one".to_owned())
            .unwrap();
        assert!(
            evidence
                .approve("call-one".to_owned(), "proof-two".to_owned())
                .is_err()
        );
        assert_eq!(evidence.take("call-one").as_deref(), Some("proof-one"));
        assert_eq!(evidence.take("call-one"), None);
        assert_eq!(evidence.take("call-two"), None);
    }
}
