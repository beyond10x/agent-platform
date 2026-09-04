#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use agent_platform_api::{Operation, ProblemDocument, ROUTES, RouteSpec};
use agent_platform_app::CapabilityProfile;
use agent_platform_core::{
    ActivateRevision, Agent, AgentRevision, CreateAgent, CreateCapabilityProfile, CreateTrigger,
    PendingApproval, ResolveApproval, RevisionSpec, SubmitTask, Task, TaskEvent, Trigger,
    UpdateCapabilityProfile,
};
use schemars::JsonSchema;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

pub const EXPECTED_OPENAPI_SHA256: &str =
    "155c23e01327526310e57f444fff2b3d788105c700dd78f44af95a621342c3aa";

/// Builds the complete `OpenAPI` document as a deterministic JSON value.
///
/// # Panics
///
/// Panics when two registered Rust schemas claim the same component name with different shapes or
/// when an internally constructed path item is not an object. Both conditions are programmer
/// errors covered by the projection tests.
pub fn document() -> Value {
    let mut schemas = BTreeMap::new();
    register::<ProblemDocument>("ProblemDocument", &mut schemas);
    register::<CreateAgent>("CreateAgent", &mut schemas);
    register::<Agent>("Agent", &mut schemas);
    register::<Vec<Agent>>("AgentList", &mut schemas);
    register::<RevisionSpec>("RevisionSpec", &mut schemas);
    register::<AgentRevision>("AgentRevision", &mut schemas);
    register::<Vec<AgentRevision>>("AgentRevisionList", &mut schemas);
    register::<ActivateRevision>("ActivateRevision", &mut schemas);
    register::<CreateCapabilityProfile>("CreateCapabilityProfile", &mut schemas);
    register::<UpdateCapabilityProfile>("UpdateCapabilityProfile", &mut schemas);
    register::<CapabilityProfile>("CapabilityProfile", &mut schemas);
    register::<Vec<CapabilityProfile>>("CapabilityProfileList", &mut schemas);
    register::<SubmitTask>("SubmitTask", &mut schemas);
    register::<Task>("Task", &mut schemas);
    register::<Vec<Task>>("TaskList", &mut schemas);
    register::<TaskEvent>("TaskEvent", &mut schemas);
    register::<PendingApproval>("PendingApproval", &mut schemas);
    register::<Vec<PendingApproval>>("PendingApprovalList", &mut schemas);
    register::<ResolveApproval>("ResolveApproval", &mut schemas);
    register::<CreateTrigger>("CreateTrigger", &mut schemas);
    register::<Trigger>("Trigger", &mut schemas);
    register::<Vec<Trigger>>("TriggerList", &mut schemas);

    let mut paths = Map::new();
    for route in ROUTES {
        let path = paths
            .entry(route.path.to_owned())
            .or_insert_with(|| Value::Object(Map::new()));
        path.as_object_mut()
            .expect("path item is an object")
            .insert(route.method.as_str().to_ascii_lowercase(), operation(route));
    }

    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Agent Platform API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Authenticated, tenant-scoped management of durable agents, immutable revisions, projected capabilities, tasks, streamed execution evidence and triggers."
        },
        "servers": [{ "url": "/" }],
        "paths": paths,
        "components": {
            "securitySchemes": {
                "bearerAuth": { "type": "http", "scheme": "bearer" }
            },
            "schemas": schemas
        },
        "x-agent-platform-route-count": ROUTES.len()
    })
}

/// Serializes [`document`] as stable pretty JSON with one trailing line feed.
///
/// # Panics
///
/// Panics only if the internally generated JSON value cannot be serialized.
pub fn document_bytes() -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(&document()).expect("OpenAPI projection serializes");
    bytes.push(b'\n');
    bytes
}

/// Returns the lowercase SHA-256 digest of [`document_bytes`].
pub fn document_sha256() -> String {
    hex::encode(Sha256::digest(document_bytes()))
}

fn operation(route: &RouteSpec) -> Value {
    let mut operation = json!({
        "operationId": route.operation.id(),
        "summary": route.operation.summary(),
        "parameters": path_parameters(route.path),
        "responses": responses(route),
        "security": if route.authenticated { json!([{ "bearerAuth": [] }]) } else { json!([]) }
    });
    if let Some(request) = request_schema(route.operation) {
        operation
            .as_object_mut()
            .expect("operation is an object")
            .insert(
                "requestBody".to_owned(),
                json!({
                    "required": true,
                    "content": { "application/json": { "schema": schema_ref(request) } }
                }),
            );
    }
    operation
}

fn responses(route: &RouteSpec) -> Value {
    let mut responses = Map::new();
    let success = if route.operation == Operation::Liveness {
        json!({
            "description": "The service process is alive.",
            "content": { "text/plain": { "schema": { "type": "string", "const": "ok\n" } } }
        })
    } else if route.operation == Operation::StreamTaskEvents {
        json!({
            "description": "A server-sent stream of ordered task events, ending at a terminal event.",
            "content": { "text/event-stream": { "schema": schema_ref("TaskEvent") } }
        })
    } else {
        json!({
            "description": "The request was answered.",
            "content": { "application/json": { "schema": schema_ref(response_schema(route.operation)) } }
        })
    };
    responses.insert(route.success_status.to_string(), success);
    if route.authenticated {
        let problem = || {
            json!({
                "description": "A named request refusal or service failure.",
                "content": { "application/json": { "schema": schema_ref("ProblemDocument") } }
            })
        };
        for status in [400, 401, 403, 404, 409, 422, 503] {
            responses.insert(status.to_string(), problem());
        }
    }
    Value::Object(responses)
}

const fn request_schema(operation: Operation) -> Option<&'static str> {
    match operation {
        Operation::CreateAgent => Some("CreateAgent"),
        Operation::CreateRevision => Some("RevisionSpec"),
        Operation::ActivateRevision => Some("ActivateRevision"),
        Operation::CreateCapabilityProfile => Some("CreateCapabilityProfile"),
        Operation::UpdateCapabilityProfile => Some("UpdateCapabilityProfile"),
        Operation::SubmitTask | Operation::SubmitCodingSessionTurn => Some("SubmitTask"),
        Operation::ResolveTaskApproval => Some("ResolveApproval"),
        Operation::CreateTrigger => Some("CreateTrigger"),
        Operation::Liveness
        | Operation::ListAgents
        | Operation::GetAgent
        | Operation::ListRevisions
        | Operation::ListCapabilityProfiles
        | Operation::ListTasks
        | Operation::GetTask
        | Operation::StreamTaskEvents
        | Operation::ListTaskApprovals
        | Operation::ListTriggers => None,
    }
}

const fn response_schema(operation: Operation) -> &'static str {
    match operation {
        Operation::ListAgents => "AgentList",
        Operation::CreateAgent | Operation::GetAgent | Operation::ActivateRevision => "Agent",
        Operation::ListRevisions => "AgentRevisionList",
        Operation::CreateRevision => "AgentRevision",
        Operation::ListCapabilityProfiles => "CapabilityProfileList",
        Operation::CreateCapabilityProfile | Operation::UpdateCapabilityProfile => {
            "CapabilityProfile"
        }
        Operation::ListTasks => "TaskList",
        Operation::SubmitTask | Operation::SubmitCodingSessionTurn | Operation::GetTask => "Task",
        Operation::StreamTaskEvents => "TaskEvent",
        Operation::ListTaskApprovals => "PendingApprovalList",
        Operation::ResolveTaskApproval => "PendingApproval",
        Operation::ListTriggers => "TriggerList",
        Operation::CreateTrigger => "Trigger",
        Operation::Liveness => "ProblemDocument",
    }
}

fn path_parameters(path: &str) -> Vec<Value> {
    ["agent_id", "profile_id", "task_id", "approval_id"]
        .into_iter()
        .filter(|name| path.contains(&format!("{{{name}}}")))
        .map(|name| {
            json!({
                "name": name,
                "in": "path",
                "required": true,
                "schema": { "type": "string", "minLength": 1, "maxLength": 128 }
            })
        })
        .collect()
}

fn schema_ref(name: &str) -> Value {
    json!({ "$ref": format!("#/components/schemas/{name}") })
}

fn register<T: JsonSchema>(name: &str, schemas: &mut BTreeMap<String, Value>) {
    let root = schemars::schema_for!(T);
    for (definition_name, definition) in root.definitions {
        insert_schema(
            definition_name,
            rewrite_references(serde_json::to_value(definition).expect("schema serializes")),
            schemas,
        );
    }
    let mut value = serde_json::to_value(root.schema).expect("root schema serializes");
    if let Some(object) = value.as_object_mut() {
        object.remove("$schema");
    }
    insert_schema(name.to_owned(), rewrite_references(value), schemas);
}

fn insert_schema(name: String, value: Value, schemas: &mut BTreeMap<String, Value>) {
    if let Some(existing) = schemas.get(&name) {
        assert_eq!(existing, &value, "schema name {name} has two meanings");
    } else {
        schemas.insert(name, value);
    }
}

fn rewrite_references(mut value: Value) -> Value {
    match &mut value {
        Value::Object(object) => {
            object.remove("$schema");
            object.remove("title");
            for member in object.values_mut() {
                *member = rewrite_references(member.take());
            }
        }
        Value::Array(items) => {
            for item in items {
                *item = rewrite_references(item.take());
            }
        }
        Value::String(reference) if reference.starts_with("#/definitions/") => {
            *reference = reference.replacen("#/definitions/", "#/components/schemas/", 1);
        }
        _ => {}
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_route_is_present_once_with_its_operation_id_and_status() {
        let document = document();
        for route in ROUTES {
            let operation =
                &document["paths"][route.path][route.method.as_str().to_ascii_lowercase()];
            assert_eq!(operation["operationId"], route.operation.id());
            assert!(operation["responses"][route.success_status.to_string()].is_object());
            assert_eq!(
                operation["security"].as_array().unwrap().is_empty(),
                !route.authenticated
            );
        }
    }

    #[test]
    fn two_projections_are_byte_identical_and_use_component_references() {
        let first = document_bytes();
        assert_eq!(first, document_bytes());
        let text = String::from_utf8(first).unwrap();
        assert!(!text.contains("#/definitions/"));
        assert_eq!(document_sha256(), EXPECTED_OPENAPI_SHA256);
    }
}
