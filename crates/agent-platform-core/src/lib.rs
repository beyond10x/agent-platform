#![forbid(unsafe_code)]

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_ID_BYTES: usize = 128;
const MAX_NAME_BYTES: usize = 256;
const MAX_INSTRUCTIONS_BYTES: usize = 128 * 1024;
const MAX_MODEL_BYTES: usize = 256;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;
const MAX_TRIGGER_EXPRESSION_BYTES: usize = 512;
const MAX_TIMEZONE_BYTES: usize = 128;
const MAX_TASK_INPUT_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    #[error("{field} must be 1..={maximum} printable ASCII bytes")]
    InvalidIdentifier { field: &'static str, maximum: usize },
    #[error("{field} must be 1..={maximum} bytes")]
    InvalidText { field: &'static str, maximum: usize },
    #[error("task input is over the {MAX_TASK_INPUT_BYTES} byte bound")]
    TaskInputTooLarge,
    #[error("a webhook input schema must be a JSON object")]
    InvalidWebhookSchema,
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(ValidationError::InvalidIdentifier {
            field,
            maximum: MAX_ID_BYTES,
        });
    }
    Ok(())
}

fn validate_reference(
    field: &'static str,
    value: &str,
    maximum: usize,
) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > maximum
        || !value.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(ValidationError::InvalidIdentifier { field, maximum });
    }
    Ok(())
}

fn validate_text(field: &'static str, value: &str, maximum: usize) -> Result<(), ValidationError> {
    if value.trim().is_empty() || value.len() > maximum {
        return Err(ValidationError::InvalidText { field, maximum });
    }
    Ok(())
}

macro_rules! id_type {
    ($name:ident, $field:literal) => {
        #[derive(
            Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
        )]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
                let value = value.into();
                validate_identifier($field, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

id_type!(TenantId, "tenant id");
id_type!(SubjectId, "subject id");
id_type!(AgentId, "agent id");
id_type!(CapabilityProfileId, "capability profile id");
id_type!(TaskId, "task id");
id_type!(AttemptId, "attempt id");
id_type!(ApprovalId, "approval id");
id_type!(TriggerId, "trigger id");
id_type!(RequestId, "request id");
id_type!(DelegationId, "delegation id");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub id: AgentId,
    pub tenant_id: TenantId,
    pub name: String,
    pub active_revision: Option<u64>,
    pub latest_revision: u64,
    pub created_by: SubjectId,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAgent {
    pub name: String,
}

impl CreateAgent {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_text("agent name", &self.name, MAX_NAME_BYTES)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevisionSpec {
    pub instructions: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability_profile_id: Option<CapabilityProfileId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl RevisionSpec {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_text(
            "revision instructions",
            &self.instructions,
            MAX_INSTRUCTIONS_BYTES,
        )?;
        validate_text("model", &self.model, MAX_MODEL_BYTES)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentRevision {
    pub agent_id: AgentId,
    pub tenant_id: TenantId,
    pub revision: u64,
    pub spec: RevisionSpec,
    pub created_by: SubjectId,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivateRevision {
    pub revision: u64,
    pub expected_active_revision: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityMapping {
    pub operation_ref: String,
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// User-selected posture inside the current server-derived authority ceiling.
    #[serde(default)]
    pub posture: CapabilityPosture,
}

/// Effective user posture for one mapped capability.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityPosture {
    /// Expose the capability while preserving any stronger upstream approval requirement.
    #[default]
    Allow,
    /// Expose the capability but require a human approval before invocation.
    ApprovalRequired,
    /// Keep the capability visible in the profile but omit it from the executable toolset.
    Deny,
}

/// Who may discover and bind one capability profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityProfileAudience {
    /// Only the verified principal who created the profile.
    Personal,
    /// Every admitted member of the tenant; typically an operator-authored template.
    Tenant,
}

const fn default_personal_profile_audience() -> CapabilityProfileAudience {
    CapabilityProfileAudience::Personal
}

/// Connector-reported effect classification captured under the caller's current authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorEffectClass {
    ReadOnly,
    Mutating,
    Destructive,
}

/// Connector-reported minimum approval posture for an operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorApprovalPosture {
    NotRequired,
    Required,
}

/// One credential-free callable connection from a Connector operation description.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectorConnectionSummary {
    pub connection_ref: String,
    pub label: String,
    pub provider: String,
    #[serde(default)]
    pub audiences: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

/// Credential-free Connector descriptions used to compile one immutable profile revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectorOperationDescription {
    pub operation_ref: String,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub effect: ConnectorEffectClass,
    pub approval: ConnectorApprovalPosture,
    pub connections: Vec<ConnectorConnectionSummary>,
    pub description_ref: String,
}

impl ConnectorOperationDescription {
    fn validate(&self) -> Result<(), ValidationError> {
        validate_identifier("operation ref", &self.operation_ref)?;
        validate_text("operation title", &self.title, 512)?;
        validate_text("operation description", &self.description, 16 * 1024)?;
        validate_identifier("description ref", &self.description_ref)?;
        if !self.input_schema.is_object() || !self.output_schema.is_object() {
            return Err(ValidationError::InvalidWebhookSchema);
        }
        if self.connections.is_empty() || self.connections.len() > 64 {
            return Err(ValidationError::InvalidText {
                field: "operation connections",
                maximum: 64,
            });
        }
        for connection in &self.connections {
            validate_identifier("connection ref", &connection.connection_ref)?;
            validate_text("connection label", &connection.label, 512)?;
            validate_identifier("connection provider", &connection.provider)?;
        }
        Ok(())
    }
}

impl CapabilityMapping {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_identifier("operation ref", &self.operation_ref)?;
        validate_identifier("tool name", &self.tool_name)?;
        if let Some(connection_ref) = &self.connection_ref {
            validate_identifier("connection ref", connection_ref)?;
        }
        if let Some(context) = &self.context {
            validate_text("mapping context", context, 2_048)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCapabilityProfile {
    pub name: String,
    /// Personal by default. Tenant profiles are explicit shared templates.
    #[serde(default = "default_personal_profile_audience")]
    pub audience: CapabilityProfileAudience,
    pub mappings: Vec<CapabilityMapping>,
    /// Exact credential-free Connector descriptions observed for this profile revision.
    #[serde(default)]
    pub operation_descriptions: Vec<ConnectorOperationDescription>,
}

/// Compare-and-swap replacement for one immutable capability-profile revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCapabilityProfile {
    pub expected_revision: u64,
    pub name: String,
    pub mappings: Vec<CapabilityMapping>,
    /// Exact credential-free Connector descriptions observed for this profile revision.
    #[serde(default)]
    pub operation_descriptions: Vec<ConnectorOperationDescription>,
}

impl CreateCapabilityProfile {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_text("capability profile name", &self.name, MAX_NAME_BYTES)?;
        if self.mappings.is_empty() || self.mappings.len() > 128 {
            return Err(ValidationError::InvalidText {
                field: "capability mappings",
                maximum: 128,
            });
        }
        self.mappings
            .iter()
            .try_for_each(CapabilityMapping::validate)?;
        self.operation_descriptions
            .iter()
            .try_for_each(ConnectorOperationDescription::validate)
    }
}

impl UpdateCapabilityProfile {
    pub fn validate(&self) -> Result<(), ValidationError> {
        CreateCapabilityProfile {
            name: self.name.clone(),
            audience: CapabilityProfileAudience::Personal,
            mappings: self.mappings.clone(),
            operation_descriptions: self.operation_descriptions.clone(),
        }
        .validate()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubmitTask {
    pub agent_id: AgentId,
    pub idempotency_key: String,
    pub input: Value,
}

/// A prior message supplied to a conversational task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConversationMessage {
    pub role: ConversationRole,
    pub content: String,
}

/// Closed role vocabulary for prior conversation context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConversationRole {
    User,
    Assistant,
    System,
}

/// One bounded text file projected from an exact repository snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectContextFile {
    pub path: String,
    pub content: String,
    pub truncated: bool,
}

/// Read-only repository context resolved by the project-context provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectContext {
    pub project_id: String,
    pub provider: String,
    pub provider_project_ref: String,
    pub path_with_namespace: String,
    pub branch: String,
    pub commit: String,
    #[serde(default)]
    pub files: Vec<ProjectContextFile>,
}

/// Typed input for one project-bound conversation turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConversationInput {
    ProjectConversation {
        prompt: String,
        #[serde(default)]
        messages: Vec<ConversationMessage>,
        context: ProjectContext,
    },
}

impl SubmitTask {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_text(
            "idempotency key",
            &self.idempotency_key,
            MAX_IDEMPOTENCY_KEY_BYTES,
        )?;
        if serde_json::to_vec(&self.input).map_or(true, |bytes| bytes.len() > MAX_TASK_INPUT_BYTES)
        {
            return Err(ValidationError::TaskInputTooLarge);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Accepted,
    Running,
    AwaitingApproval,
    Succeeded,
    Failed,
    Cancelled,
    Refused,
    OutcomeUnknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub id: TaskId,
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub agent_revision: u64,
    pub capability_profile_id: Option<CapabilityProfileId>,
    pub idempotency_key: String,
    pub input: Value,
    pub status: TaskStatus,
    pub attempt_id: AttemptId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<TaskFailure>,
    pub actor: SubjectId,
    pub executor: Option<SubjectId>,
    pub delegation_id: Option<DelegationId>,
    pub request_id: RequestId,
    pub accepted_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskFailure {
    pub code: String,
    pub message: String,
}

/// Non-secret owner snapshot shared by one invocation and its human approval request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectorOwnerContext {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub agent_revision: u64,
    pub authority_snapshot_id: RequestId,
    pub authority_snapshot_sha256: String,
}

/// Exact effectful Connector call suspended for a human decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PendingApproval {
    pub id: ApprovalId,
    pub task_id: TaskId,
    pub attempt_id: AttemptId,
    pub call_id: String,
    pub tool_name: String,
    pub operation_ref: String,
    pub connection_ref: String,
    pub description_ref: String,
    pub input: Value,
    pub context: ConnectorOwnerContext,
    pub requested_at_ms: u64,
}

/// Human decision submitted to the suspended attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "decision", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResolveApproval {
    Approve { approval_evidence_ref: String },
    Deny { reason: String },
}

impl ResolveApproval {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Approve {
                approval_evidence_ref,
            } => validate_reference("approval evidence reference", approval_evidence_ref, 512),
            Self::Deny { reason } => validate_text("approval denial reason", reason, 4_096),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TaskEventKind {
    Accepted,
    Running,
    TextDelta {
        text: String,
    },
    ApprovalRequested {
        approval_id: ApprovalId,
        call_id: String,
        operation_ref: String,
        connection_ref: String,
    },
    ApprovalResolved {
        approval_id: ApprovalId,
        approved: bool,
    },
    Succeeded {
        output: String,
    },
    Failed {
        failure: TaskFailure,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskEvent {
    pub task_id: TaskId,
    pub attempt_id: AttemptId,
    pub sequence: u64,
    pub occurred_at_ms: u64,
    pub event: TaskEventKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MisfirePolicy {
    Skip,
    RunOnce,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OverlapPolicy {
    Skip,
    Queue,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TriggerKind {
    Schedule {
        expression: String,
        timezone: String,
        misfire: MisfirePolicy,
        overlap: OverlapPolicy,
    },
    Webhook {
        input_schema: Value,
    },
}

impl TriggerKind {
    fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Schedule {
                expression,
                timezone,
                ..
            } => {
                validate_text(
                    "schedule expression",
                    expression,
                    MAX_TRIGGER_EXPRESSION_BYTES,
                )?;
                validate_text("schedule timezone", timezone, MAX_TIMEZONE_BYTES)
            }
            Self::Webhook { input_schema } => {
                if !input_schema.is_object() {
                    return Err(ValidationError::InvalidWebhookSchema);
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateTrigger {
    pub name: String,
    pub agent_id: AgentId,
    pub enabled: bool,
    pub task_input: Value,
    pub trigger: TriggerKind,
}

impl CreateTrigger {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_text("trigger name", &self.name, MAX_NAME_BYTES)?;
        if serde_json::to_vec(&self.task_input)
            .map_or(true, |bytes| bytes.len() > MAX_TASK_INPUT_BYTES)
        {
            return Err(ValidationError::TaskInputTooLarge);
        }
        self.trigger.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Trigger {
    pub id: TriggerId,
    pub tenant_id: TenantId,
    pub name: String,
    pub agent_id: AgentId,
    pub agent_revision: u64,
    pub enabled: bool,
    pub task_input: Value,
    pub trigger: TriggerKind,
    pub authority_subject: SubjectId,
    pub delegation_id: Option<DelegationId>,
    pub created_at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_intent_is_bounded_before_storage() {
        let task = SubmitTask {
            agent_id: AgentId::new("agent-1").unwrap(),
            idempotency_key: "retry-1".to_owned(),
            input: Value::String("x".repeat(MAX_TASK_INPUT_BYTES)),
        };
        assert_eq!(task.validate(), Err(ValidationError::TaskInputTooLarge));
    }

    #[test]
    fn webhook_schema_must_be_an_object() {
        let trigger = TriggerKind::Webhook {
            input_schema: Value::String("anything".to_owned()),
        };
        assert_eq!(
            trigger.validate(),
            Err(ValidationError::InvalidWebhookSchema)
        );
    }
}
