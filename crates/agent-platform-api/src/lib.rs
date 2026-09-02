#![forbid(unsafe_code)]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const LIVENESS_PATH: &str = "/livez";
pub const AGENTS_PATH: &str = "/v1/agents";
pub const AGENT_PATH: &str = "/v1/agents/{agent_id}";
pub const REVISIONS_PATH: &str = "/v1/agents/{agent_id}/revisions";
pub const ACTIVATE_PATH: &str = "/v1/agents/{agent_id}/activate";
pub const CAPABILITY_PROFILES_PATH: &str = "/v1/capability-profiles";
pub const CAPABILITY_PROFILE_PATH: &str = "/v1/capability-profiles/{profile_id}";
pub const TASKS_PATH: &str = "/v1/tasks";
pub const TASK_PATH: &str = "/v1/tasks/{task_id}";
pub const TASK_EVENTS_PATH: &str = "/v1/tasks/{task_id}/events";
pub const TASK_APPROVALS_PATH: &str = "/v1/tasks/{task_id}/approvals";
pub const TASK_APPROVAL_PATH: &str = "/v1/tasks/{task_id}/approvals/{approval_id}";
pub const TRIGGERS_PATH: &str = "/v1/triggers";
pub const OPENAPI_PATH: &str = "/openapi.json";
pub const DOCS_ROOT_PATH: &str = "/docs";
pub const DOCS_INDEX_PATH: &str = "/docs/";
pub const DOCS_API_PATH: &str = "/docs/api/";
pub const DOCS_STYLES_PATH: &str = "/docs/styles.css";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Method {
    Get,
    Post,
    Patch,
}

impl Method {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Patch => "PATCH",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Operation {
    Liveness,
    ListAgents,
    CreateAgent,
    GetAgent,
    ListRevisions,
    CreateRevision,
    ActivateRevision,
    ListCapabilityProfiles,
    CreateCapabilityProfile,
    UpdateCapabilityProfile,
    ListTasks,
    SubmitTask,
    GetTask,
    StreamTaskEvents,
    ListTaskApprovals,
    ResolveTaskApproval,
    ListTriggers,
    CreateTrigger,
}

impl Operation {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Liveness => "getLiveness",
            Self::ListAgents => "listAgents",
            Self::CreateAgent => "createAgent",
            Self::GetAgent => "getAgent",
            Self::ListRevisions => "listAgentRevisions",
            Self::CreateRevision => "createAgentRevision",
            Self::ActivateRevision => "activateAgentRevision",
            Self::ListCapabilityProfiles => "listCapabilityProfiles",
            Self::CreateCapabilityProfile => "createCapabilityProfile",
            Self::UpdateCapabilityProfile => "updateCapabilityProfile",
            Self::ListTasks => "listTasks",
            Self::SubmitTask => "submitTask",
            Self::GetTask => "getTask",
            Self::StreamTaskEvents => "streamTaskEvents",
            Self::ListTaskApprovals => "listTaskApprovals",
            Self::ResolveTaskApproval => "resolveTaskApproval",
            Self::ListTriggers => "listTriggers",
            Self::CreateTrigger => "createTrigger",
        }
    }

    pub const fn summary(self) -> &'static str {
        match self {
            Self::Liveness => "Check process liveness",
            Self::ListAgents => "List agents",
            Self::CreateAgent => "Create an agent",
            Self::GetAgent => "Get an agent",
            Self::ListRevisions => "List immutable agent revisions",
            Self::CreateRevision => "Create an immutable agent revision",
            Self::ActivateRevision => "Activate an agent revision with compare-and-swap",
            Self::ListCapabilityProfiles => "List capability profiles",
            Self::CreateCapabilityProfile => "Compile a Connector capability profile",
            Self::UpdateCapabilityProfile => "Replace a capability profile with compare-and-swap",
            Self::ListTasks => "List admitted tasks",
            Self::SubmitTask => "Idempotently admit an asynchronous task",
            Self::GetTask => "Get task state",
            Self::StreamTaskEvents => "Stream task events",
            Self::ListTaskApprovals => "List exact Connector calls awaiting human approval",
            Self::ResolveTaskApproval => "Resolve an exact Connector call approval",
            Self::ListTriggers => "List trigger definitions",
            Self::CreateTrigger => "Create a schedule or webhook trigger definition",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RouteSpec {
    pub method: Method,
    pub path: &'static str,
    pub operation: Operation,
    pub authenticated: bool,
    pub success_status: u16,
}

pub const ROUTES: &[RouteSpec] = &[
    RouteSpec {
        method: Method::Get,
        path: LIVENESS_PATH,
        operation: Operation::Liveness,
        authenticated: false,
        success_status: 200,
    },
    RouteSpec {
        method: Method::Get,
        path: AGENTS_PATH,
        operation: Operation::ListAgents,
        authenticated: true,
        success_status: 200,
    },
    RouteSpec {
        method: Method::Post,
        path: AGENTS_PATH,
        operation: Operation::CreateAgent,
        authenticated: true,
        success_status: 201,
    },
    RouteSpec {
        method: Method::Get,
        path: AGENT_PATH,
        operation: Operation::GetAgent,
        authenticated: true,
        success_status: 200,
    },
    RouteSpec {
        method: Method::Get,
        path: REVISIONS_PATH,
        operation: Operation::ListRevisions,
        authenticated: true,
        success_status: 200,
    },
    RouteSpec {
        method: Method::Post,
        path: REVISIONS_PATH,
        operation: Operation::CreateRevision,
        authenticated: true,
        success_status: 201,
    },
    RouteSpec {
        method: Method::Post,
        path: ACTIVATE_PATH,
        operation: Operation::ActivateRevision,
        authenticated: true,
        success_status: 200,
    },
    RouteSpec {
        method: Method::Get,
        path: CAPABILITY_PROFILES_PATH,
        operation: Operation::ListCapabilityProfiles,
        authenticated: true,
        success_status: 200,
    },
    RouteSpec {
        method: Method::Post,
        path: CAPABILITY_PROFILES_PATH,
        operation: Operation::CreateCapabilityProfile,
        authenticated: true,
        success_status: 201,
    },
    RouteSpec {
        method: Method::Patch,
        path: CAPABILITY_PROFILE_PATH,
        operation: Operation::UpdateCapabilityProfile,
        authenticated: true,
        success_status: 200,
    },
    RouteSpec {
        method: Method::Get,
        path: TASKS_PATH,
        operation: Operation::ListTasks,
        authenticated: true,
        success_status: 200,
    },
    RouteSpec {
        method: Method::Post,
        path: TASKS_PATH,
        operation: Operation::SubmitTask,
        authenticated: true,
        success_status: 202,
    },
    RouteSpec {
        method: Method::Get,
        path: TASK_PATH,
        operation: Operation::GetTask,
        authenticated: true,
        success_status: 200,
    },
    RouteSpec {
        method: Method::Get,
        path: TASK_EVENTS_PATH,
        operation: Operation::StreamTaskEvents,
        authenticated: true,
        success_status: 200,
    },
    RouteSpec {
        method: Method::Get,
        path: TASK_APPROVALS_PATH,
        operation: Operation::ListTaskApprovals,
        authenticated: true,
        success_status: 200,
    },
    RouteSpec {
        method: Method::Post,
        path: TASK_APPROVAL_PATH,
        operation: Operation::ResolveTaskApproval,
        authenticated: true,
        success_status: 202,
    },
    RouteSpec {
        method: Method::Get,
        path: TRIGGERS_PATH,
        operation: Operation::ListTriggers,
        authenticated: true,
        success_status: 200,
    },
    RouteSpec {
        method: Method::Post,
        path: TRIGGERS_PATH,
        operation: Operation::CreateTrigger,
        authenticated: true,
        success_status: 201,
    },
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProblemDocument {
    pub code: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn method_path_and_operation_id_are_unique() {
        let mut routes = BTreeSet::new();
        let mut operations = BTreeSet::new();
        for route in ROUTES {
            assert!(routes.insert((route.method, route.path)));
            assert!(operations.insert(route.operation.id()));
        }
    }
}
