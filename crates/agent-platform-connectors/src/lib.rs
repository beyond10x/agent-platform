#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;

use agent_platform_core::{CapabilityMapping, TenantId};
use harness_wire::{AccessKind, Approval, Effect, Envelope, Idempotency, Risk, ToolName, ToolSpec};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const CONNECTOR_OPERATION_CONTRACT: &str = "b10x.connector-operation.v0alpha1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectClass {
    ReadOnly,
    Mutating,
    Destructive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPosture {
    NotRequired,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionSummary {
    pub connection_ref: String,
    pub label: String,
    pub provider: String,
    #[serde(default)]
    pub audiences: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDescription {
    pub operation_ref: String,
    pub title: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub effect: EffectClass,
    pub approval: ApprovalPosture,
    pub connections: Vec<ConnectionSummary>,
    pub description_ref: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompiledCapability {
    pub operation_ref: String,
    pub connection_ref: String,
    pub description_ref: String,
    #[schemars(with = "schema::ToolSpec")]
    pub tool: ToolSpec,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompiledToolset {
    pub connector_contract: String,
    pub digest_sha256: String,
    pub capabilities: Vec<CompiledCapability>,
}

#[allow(dead_code)]
mod schema {
    use schemars::JsonSchema;
    use serde_json::Value;

    #[derive(JsonSchema)]
    #[schemars(rename = "HarnessToolSpec")]
    pub struct ToolSpec {
        pub name: String,
        pub description: String,
        pub input_schema: Value,
        pub approval: Approval,
        pub envelope: Envelope,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "kebab-case")]
    pub enum Approval {
        NotRequired,
        Required,
    }

    #[derive(JsonSchema)]
    pub struct Envelope {
        pub effects: Vec<Effect>,
        pub risk: Risk,
        pub idempotency: Idempotency,
        pub access: Vec<AccessKind>,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum Effect {
        Read,
        Write,
        Network,
        Process,
        Filesystem,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum Risk {
        Low,
        Medium,
        High,
        Destructive,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum Idempotency {
        Idempotent,
        NonIdempotent,
        Conditional,
    }

    #[derive(JsonSchema)]
    #[schemars(rename_all = "snake_case")]
    pub enum AccessKind {
        Filesystem,
        Process,
        Network,
        Secret,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionError {
    #[error("Connector operation `{0}` was not found")]
    OperationNotFound(String),
    #[error("Connector operation `{operation_ref}` has no callable connection")]
    NoConnection { operation_ref: String },
    #[error(
        "Connector operation `{operation_ref}` has multiple connections; the mapping must select one"
    )]
    AmbiguousConnection { operation_ref: String },
    #[error(
        "connection `{connection_ref}` is not callable for Connector operation `{operation_ref}`"
    )]
    UnknownConnection {
        operation_ref: String,
        connection_ref: String,
    },
    #[error(
        "Connector operation `{operation_ref}` weakens required approval for an effectful call"
    )]
    UnsafeApproval { operation_ref: String },
    #[error("tool name `{0}` appears more than once in the capability profile")]
    DuplicateToolName(String),
    #[error("tool name `{0}` is not a valid Harness tool name")]
    InvalidToolName(String),
    #[error("capability projection could not be serialized deterministically")]
    Serialization,
}

pub trait ConnectorCatalog: Send + Sync {
    fn describe<'a>(
        &'a self,
        tenant_id: &'a TenantId,
        operation_ref: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<OperationDescription, ProjectionError>> + Send + 'a>>;
}

#[derive(Debug, Default)]
pub struct EmptyCatalog;

impl ConnectorCatalog for EmptyCatalog {
    fn describe<'a>(
        &'a self,
        _tenant_id: &'a TenantId,
        operation_ref: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<OperationDescription, ProjectionError>> + Send + 'a>>
    {
        Box::pin(async move { Err(ProjectionError::OperationNotFound(operation_ref.to_owned())) })
    }
}

#[derive(Debug, Clone, Default)]
pub struct InMemoryCatalog {
    operations: BTreeMap<String, OperationDescription>,
}

impl InMemoryCatalog {
    pub fn new(operations: impl IntoIterator<Item = OperationDescription>) -> Self {
        Self {
            operations: operations
                .into_iter()
                .map(|operation| (operation.operation_ref.clone(), operation))
                .collect(),
        }
    }
}

impl ConnectorCatalog for InMemoryCatalog {
    fn describe<'a>(
        &'a self,
        _tenant_id: &'a TenantId,
        operation_ref: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<OperationDescription, ProjectionError>> + Send + 'a>>
    {
        Box::pin(async move {
            self.operations
                .get(operation_ref)
                .cloned()
                .ok_or_else(|| ProjectionError::OperationNotFound(operation_ref.to_owned()))
        })
    }
}

pub async fn compile(
    catalog: &dyn ConnectorCatalog,
    tenant_id: &TenantId,
    mappings: &[CapabilityMapping],
) -> Result<CompiledToolset, ProjectionError> {
    let mut names = BTreeSet::new();
    let mut capabilities = Vec::with_capacity(mappings.len());
    let mut digest = Sha256::new();
    digest_field(&mut digest, CONNECTOR_OPERATION_CONTRACT.as_bytes());

    for mapping in mappings {
        if !names.insert(mapping.tool_name.clone()) {
            return Err(ProjectionError::DuplicateToolName(
                mapping.tool_name.clone(),
            ));
        }
        let operation = catalog.describe(tenant_id, &mapping.operation_ref).await?;
        if operation.operation_ref != mapping.operation_ref {
            return Err(ProjectionError::OperationNotFound(
                mapping.operation_ref.clone(),
            ));
        }
        if operation.effect != EffectClass::ReadOnly
            && operation.approval != ApprovalPosture::Required
        {
            return Err(ProjectionError::UnsafeApproval {
                operation_ref: operation.operation_ref,
            });
        }
        let connection_ref = selected_connection(mapping, &operation)?;
        let tool_name = ToolName::new(mapping.tool_name.clone())
            .map_err(|_| ProjectionError::InvalidToolName(mapping.tool_name.clone()))?;
        let description = mapping.context.as_ref().map_or_else(
            || operation.description.clone(),
            |context| format!("{}\n\nAgent context: {context}", operation.description),
        );
        let envelope = envelope(operation.effect);
        let approval = match operation.approval {
            ApprovalPosture::NotRequired => Approval::NotRequired,
            ApprovalPosture::Required => Approval::Required,
        };
        let tool = ToolSpec {
            name: tool_name,
            description,
            input_schema: operation.input_schema.clone(),
            approval,
            envelope,
        };
        let compiled = CompiledCapability {
            operation_ref: operation.operation_ref.clone(),
            connection_ref,
            description_ref: operation.description_ref.clone(),
            tool,
        };
        let mapping_bytes =
            serde_json::to_vec(mapping).map_err(|_| ProjectionError::Serialization)?;
        let operation_bytes =
            serde_json::to_vec(&operation).map_err(|_| ProjectionError::Serialization)?;
        let compiled_bytes =
            serde_json::to_vec(&compiled).map_err(|_| ProjectionError::Serialization)?;
        digest_field(&mut digest, &mapping_bytes);
        digest_field(&mut digest, &operation_bytes);
        digest_field(&mut digest, &compiled_bytes);
        capabilities.push(compiled);
    }

    Ok(CompiledToolset {
        connector_contract: CONNECTOR_OPERATION_CONTRACT.to_owned(),
        digest_sha256: hex::encode(digest.finalize()),
        capabilities,
    })
}

fn selected_connection(
    mapping: &CapabilityMapping,
    operation: &OperationDescription,
) -> Result<String, ProjectionError> {
    if let Some(selected) = &mapping.connection_ref {
        return operation
            .connections
            .iter()
            .find(|connection| connection.connection_ref == *selected)
            .map(|connection| connection.connection_ref.clone())
            .ok_or_else(|| ProjectionError::UnknownConnection {
                operation_ref: operation.operation_ref.clone(),
                connection_ref: selected.clone(),
            });
    }
    match operation.connections.as_slice() {
        [] => Err(ProjectionError::NoConnection {
            operation_ref: operation.operation_ref.clone(),
        }),
        [only] => Ok(only.connection_ref.clone()),
        _ => Err(ProjectionError::AmbiguousConnection {
            operation_ref: operation.operation_ref.clone(),
        }),
    }
}

fn envelope(effect: EffectClass) -> Envelope {
    match effect {
        EffectClass::ReadOnly => Envelope {
            effects: vec![Effect::Read],
            risk: Risk::Low,
            idempotency: Idempotency::Idempotent,
            access: vec![AccessKind::Network],
        },
        EffectClass::Mutating => Envelope {
            effects: vec![Effect::Write],
            risk: Risk::High,
            idempotency: Idempotency::NonIdempotent,
            access: vec![AccessKind::Network],
        },
        EffectClass::Destructive => Envelope {
            effects: vec![Effect::Write],
            risk: Risk::Destructive,
            idempotency: Idempotency::NonIdempotent,
            access: vec![AccessKind::Network],
        },
    }
}

fn digest_field(digest: &mut Sha256, bytes: &[u8]) {
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn operation(effect: EffectClass, approval: ApprovalPosture) -> OperationDescription {
        OperationDescription {
            operation_ref: "tickets.create".to_owned(),
            title: "Create ticket".to_owned(),
            description: "Create one support ticket.".to_owned(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["subject"],
                "properties": {"subject": {"type": "string"}},
                "additionalProperties": false
            }),
            output_schema: serde_json::json!({"type": "object"}),
            effect,
            approval,
            connections: vec![ConnectionSummary {
                connection_ref: "connection-support".to_owned(),
                label: "Support".to_owned(),
                provider: "zendesk".to_owned(),
                audiences: vec!["support".to_owned()],
                purpose: None,
            }],
            description_ref: "description-1".to_owned(),
        }
    }

    fn mapping() -> CapabilityMapping {
        CapabilityMapping {
            operation_ref: "tickets.create".to_owned(),
            tool_name: "create_support_ticket".to_owned(),
            connection_ref: None,
            context: Some("Use only after troubleshooting is complete.".to_owned()),
        }
    }

    #[tokio::test]
    async fn projection_is_deterministic_and_conservatively_preserves_effects() {
        let catalog =
            InMemoryCatalog::new([operation(EffectClass::Mutating, ApprovalPosture::Required)]);
        let tenant = TenantId::new("tenant-one").unwrap();
        let first = compile(&catalog, &tenant, &[mapping()]).await.unwrap();
        let second = compile(&catalog, &tenant, &[mapping()]).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(first.capabilities[0].tool.envelope.risk, Risk::High);
        assert_eq!(first.capabilities[0].tool.approval, Approval::Required);
        assert_eq!(
            first.capabilities[0].tool.input_schema,
            operation(EffectClass::Mutating, ApprovalPosture::Required).input_schema
        );
    }

    #[tokio::test]
    async fn effectful_source_cannot_claim_approval_is_not_required() {
        let catalog = InMemoryCatalog::new([operation(
            EffectClass::Destructive,
            ApprovalPosture::NotRequired,
        )]);
        let error = compile(
            &catalog,
            &TenantId::new("tenant-one").unwrap(),
            &[mapping()],
        )
        .await
        .unwrap_err();
        assert!(matches!(error, ProjectionError::UnsafeApproval { .. }));
    }

    #[tokio::test]
    async fn duplicate_agent_tool_names_are_refused() {
        let catalog = InMemoryCatalog::new([operation(
            EffectClass::ReadOnly,
            ApprovalPosture::NotRequired,
        )]);
        let error = compile(
            &catalog,
            &TenantId::new("tenant-one").unwrap(),
            &[mapping(), mapping()],
        )
        .await
        .unwrap_err();
        assert_eq!(
            error,
            ProjectionError::DuplicateToolName("create_support_ticket".to_owned())
        );
    }
}
