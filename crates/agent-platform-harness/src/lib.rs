#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use agent_platform_auth::UserModelLease;
use agent_platform_connectors::{CompiledCapability, CompiledToolset};
use agent_platform_core::RevisionSpec;
use harness_loop::{AgentLoop, DenyAll, LoopConfig, LoopError, LoopEvent, LoopOutcome, LoopSink};
use harness_messages::{Endpoint, MessagesClient};
use harness_wire::{
    Bearer, BearerSource, CredentialKind, ModelPort, Subject, ToolCall, ToolOutcome, ToolPort,
    ToolSpec, WireError,
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
                .map_err(|_| ExecutionError::ModelUnavailable)?;
            let toolset = toolset.unwrap_or_else(empty_toolset);
            let mut tools = ConnectorTools::new(&toolset, Arc::new(RefusingInvoker));
            let mut sink = TextSink { emit_text };
            let outcome = run(&mut model, &mut tools, &revision, prompt, &mut sink)
                .map_err(|_| ExecutionError::ModelUnavailable)?;
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
    #[error("the user-bound model is unavailable")]
    ModelUnavailable,
    #[error("the Harness run stopped before completing")]
    Incomplete,
    #[error("the execution worker is unavailable")]
    WorkerUnavailable,
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
