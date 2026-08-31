#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use agent_platform_connectors::{CompiledCapability, CompiledToolset};
use agent_platform_core::RevisionSpec;
use harness_loop::{AgentLoop, DenyAll, LoopConfig, LoopError, LoopOutcome, LoopSink};
use harness_wire::{ModelPort, Subject, ToolCall, ToolOutcome, ToolPort, ToolSpec};
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
