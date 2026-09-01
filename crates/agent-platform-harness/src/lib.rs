#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use agent_platform_auth::UserModelLease;
use agent_platform_connectors::{CompiledCapability, CompiledToolset};
use agent_platform_core::{ConversationInput, ConversationRole, RevisionSpec};
use harness_loop::{AgentLoop, DenyAll, LoopConfig, LoopError, LoopEvent, LoopOutcome, LoopSink};
use harness_messages::{Endpoint, MessagesClient};
use harness_wire::{
    Bearer, BearerSource, CredentialKind, ModelPort, Subject, ToolCall, ToolOutcome, ToolPort,
    ToolSpec, WireError, WireErrorCode,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectorInvocation {
    pub operation_ref: String,
    pub connection_ref: String,
    pub description_ref: String,
    pub input: Value,
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
            operation_ref: capability.operation_ref.clone(),
            connection_ref: capability.connection_ref.clone(),
            description_ref: capability.description_ref.clone(),
            input: call.arguments.clone(),
        })
    }
}

pub fn run(
    model: &mut dyn ModelPort,
    tools: &mut ConnectorTools,
    revision: &RevisionSpec,
    input: impl Into<String>,
    sink: &mut dyn LoopSink,
) -> Result<LoopOutcome, LoopError> {
    let mut approvals = DenyAll;
    AgentLoop::new(
        model,
        tools,
        &mut approvals,
        LoopConfig::new(revision.model.clone(), revision.instructions.clone()),
    )
    .run(input, sink)
}

#[derive(Debug, Clone)]
pub struct UserModelRunner {
    endpoint_base: String,
    context_window: u64,
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
        revision: RevisionSpec,
        toolset: Option<CompiledToolset>,
        input: Value,
        lease: Arc<UserModelLease>,
        emit_text: Arc<dyn Fn(String) + Send + Sync>,
    ) -> Result<String, ExecutionError> {
        let prompt = task_prompt(&input)?;
        let endpoint_base = self.endpoint_base.clone();
        let context_window = self.context_window;
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            let endpoint = Endpoint::new(endpoint_base, revision.model.clone(), context_window)
                .map_err(|_| ExecutionError::Configuration)?;
            let source = Arc::new(LeaseBearerSource { lease, handle });
            let mut model = MessagesClient::new(endpoint, source)
                .map_err(|error| execution_wire_error(&error))?;
            let toolset = toolset.unwrap_or_else(empty_toolset);
            let mut tools = ConnectorTools::new(&toolset, Arc::new(RefusingInvoker));
            let mut sink = TextSink { emit_text };
            let outcome = run(&mut model, &mut tools, &revision, prompt, &mut sink)
                .map_err(|error| execution_loop_error(&error))?;
            if outcome.stop.is_completed() {
                Ok(outcome.text)
            } else {
                Err(ExecutionError::Incomplete)
            }
        })
        .await
        .map_err(|_| ExecutionError::WorkerUnavailable)?
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
            Self::WorkerUnavailable => "execution_worker_unavailable",
        }
    }
}

fn execution_loop_error(error: &LoopError) -> ExecutionError {
    match error {
        LoopError::Wire(error) => execution_wire_error(error),
        LoopError::Budget(_) => ExecutionError::RequestTooLarge,
        LoopError::Config(_) => ExecutionError::HarnessConfiguration,
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

struct TextSink {
    emit_text: Arc<dyn Fn(String) + Send + Sync>,
}

impl LoopSink for TextSink {
    fn emit(&mut self, event: LoopEvent) {
        if let LoopEvent::TextDelta { text } = event {
            (self.emit_text)(text);
        }
    }
}

#[derive(Debug)]
struct RefusingInvoker;

impl ConnectorInvoker for RefusingInvoker {
    fn invoke(&self, _: ConnectorInvocation) -> ToolOutcome {
        ToolOutcome::failed("Connector invocation is not configured for this worker")
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
        let outcome = run(
            &mut model,
            &mut tools,
            &revision,
            "List open support work.",
            &mut sink,
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
}
