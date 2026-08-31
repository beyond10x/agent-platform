#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use agent_platform_auth::{
    AGENTS_MANAGE, AGENTS_READ, CAPABILITIES_MANAGE, CAPABILITIES_READ, TASKS_READ, TASKS_SUBMIT,
    TRIGGERS_MANAGE, TRIGGERS_READ, VerifiedAuthority,
};
use agent_platform_connectors::{CompiledToolset, ConnectorCatalog, ProjectionError, compile};
use agent_platform_core::{
    ActivateRevision, Agent, AgentId, AgentRevision, CapabilityProfileId, CreateAgent,
    CreateCapabilityProfile, CreateTrigger, RequestId, RevisionSpec, SubmitTask, Task, TaskId,
    TaskStatus, TenantId, Trigger, TriggerId, ValidationError,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedRequestContext {
    authority: VerifiedAuthority,
    request_id: RequestId,
    received_at_ms: u64,
}

impl TrustedRequestContext {
    pub fn new(authority: VerifiedAuthority, request_id: RequestId, received_at_ms: u64) -> Self {
        Self {
            authority,
            request_id,
            received_at_ms,
        }
    }

    pub fn authority(&self) -> &VerifiedAuthority {
        &self.authority
    }

    pub fn request_id(&self) -> &RequestId {
        &self.request_id
    }

    pub const fn received_at_ms(&self) -> u64 {
        self.received_at_ms
    }

    fn require(&self, scope: &'static str) -> Result<(), ApplicationError> {
        if self.authority.permits(scope) {
            Ok(())
        } else {
            Err(ApplicationError::Forbidden { scope })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityProfile {
    pub id: CapabilityProfileId,
    pub tenant_id: TenantId,
    pub name: String,
    pub mappings: Vec<agent_platform_core::CapabilityMapping>,
    pub compiled: CompiledToolset,
    pub created_by: agent_platform_core::SubjectId,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApplicationError {
    #[error("the verified authority lacks `{scope}`")]
    Forbidden { scope: &'static str },
    #[error("agent was not found")]
    AgentNotFound,
    #[error("agent revision was not found")]
    RevisionNotFound,
    #[error("capability profile was not found")]
    CapabilityProfileNotFound,
    #[error("task was not found")]
    TaskNotFound,
    #[error("agent has no active revision")]
    NoActiveRevision,
    #[error("active revision changed; expected {expected:?}, found {actual:?}")]
    ActiveRevisionConflict {
        expected: Option<u64>,
        actual: Option<u64>,
    },
    #[error("idempotency key was already used for different task intent")]
    IdempotencyConflict,
    #[error("application state lock is unavailable")]
    StateUnavailable,
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    #[error(transparent)]
    Projection(#[from] ProjectionError),
}

#[derive(Debug, Default)]
struct State {
    tenants: BTreeMap<TenantId, TenantState>,
}

#[derive(Debug, Default)]
struct TenantState {
    agents: BTreeMap<AgentId, Agent>,
    revisions: BTreeMap<AgentId, BTreeMap<u64, AgentRevision>>,
    profiles: BTreeMap<CapabilityProfileId, CapabilityProfile>,
    tasks: BTreeMap<TaskId, Task>,
    task_keys: BTreeMap<String, TaskId>,
    triggers: BTreeMap<TriggerId, Trigger>,
}

#[derive(Clone)]
pub struct Application {
    state: Arc<Mutex<State>>,
    catalog: Arc<dyn ConnectorCatalog>,
}

impl std::fmt::Debug for Application {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Application")
            .field("state", &"tenant-scoped")
            .field("catalog", &"ConnectorCatalog")
            .finish()
    }
}

impl Application {
    pub fn new(catalog: Arc<dyn ConnectorCatalog>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            catalog,
        }
    }

    pub fn create_agent(
        &self,
        context: &TrustedRequestContext,
        request: CreateAgent,
    ) -> Result<Agent, ApplicationError> {
        context.require(AGENTS_MANAGE)?;
        request.validate()?;
        let agent = Agent {
            id: new_agent_id()?,
            tenant_id: context.authority.tenant_id().clone(),
            name: request.name,
            active_revision: None,
            latest_revision: 0,
            created_by: context.authority.authority().clone(),
            created_at_ms: context.received_at_ms,
        };
        let mut state = self.lock_state()?;
        state
            .tenants
            .entry(context.authority.tenant_id().clone())
            .or_default()
            .agents
            .insert(agent.id.clone(), agent.clone());
        Ok(agent)
    }

    pub fn list_agents(
        &self,
        context: &TrustedRequestContext,
    ) -> Result<Vec<Agent>, ApplicationError> {
        context.require(AGENTS_READ)?;
        let state = self.lock_state()?;
        Ok(state
            .tenants
            .get(context.authority.tenant_id())
            .map_or_else(Vec::new, |tenant| tenant.agents.values().cloned().collect()))
    }

    pub fn get_agent(
        &self,
        context: &TrustedRequestContext,
        agent_id: &AgentId,
    ) -> Result<Agent, ApplicationError> {
        context.require(AGENTS_READ)?;
        let state = self.lock_state()?;
        tenant(&state, context)
            .and_then(|tenant| tenant.agents.get(agent_id))
            .cloned()
            .ok_or(ApplicationError::AgentNotFound)
    }

    pub fn create_revision(
        &self,
        context: &TrustedRequestContext,
        agent_id: &AgentId,
        spec: RevisionSpec,
    ) -> Result<AgentRevision, ApplicationError> {
        context.require(AGENTS_MANAGE)?;
        spec.validate()?;
        let mut state = self.lock_state()?;
        let tenant = tenant_mut(&mut state, context);
        if let Some(profile_id) = &spec.capability_profile_id
            && !tenant.profiles.contains_key(profile_id)
        {
            return Err(ApplicationError::CapabilityProfileNotFound);
        }
        let agent = tenant
            .agents
            .get_mut(agent_id)
            .ok_or(ApplicationError::AgentNotFound)?;
        let revision = agent
            .latest_revision
            .checked_add(1)
            .ok_or(ApplicationError::StateUnavailable)?;
        agent.latest_revision = revision;
        let revision = AgentRevision {
            agent_id: agent_id.clone(),
            tenant_id: context.authority.tenant_id().clone(),
            revision,
            spec,
            created_by: context.authority.authority().clone(),
            created_at_ms: context.received_at_ms,
        };
        tenant
            .revisions
            .entry(agent_id.clone())
            .or_default()
            .insert(revision.revision, revision.clone());
        Ok(revision)
    }

    pub fn list_revisions(
        &self,
        context: &TrustedRequestContext,
        agent_id: &AgentId,
    ) -> Result<Vec<AgentRevision>, ApplicationError> {
        context.require(AGENTS_READ)?;
        let state = self.lock_state()?;
        let tenant = tenant(&state, context).ok_or(ApplicationError::AgentNotFound)?;
        if !tenant.agents.contains_key(agent_id) {
            return Err(ApplicationError::AgentNotFound);
        }
        Ok(tenant
            .revisions
            .get(agent_id)
            .map_or_else(Vec::new, |revisions| revisions.values().cloned().collect()))
    }

    pub fn activate_revision(
        &self,
        context: &TrustedRequestContext,
        agent_id: &AgentId,
        request: &ActivateRevision,
    ) -> Result<Agent, ApplicationError> {
        context.require(AGENTS_MANAGE)?;
        let mut state = self.lock_state()?;
        let tenant = tenant_mut(&mut state, context);
        if !tenant
            .revisions
            .get(agent_id)
            .is_some_and(|revisions| revisions.contains_key(&request.revision))
        {
            return Err(ApplicationError::RevisionNotFound);
        }
        let agent = tenant
            .agents
            .get_mut(agent_id)
            .ok_or(ApplicationError::AgentNotFound)?;
        if agent.active_revision != request.expected_active_revision {
            return Err(ApplicationError::ActiveRevisionConflict {
                expected: request.expected_active_revision,
                actual: agent.active_revision,
            });
        }
        agent.active_revision = Some(request.revision);
        Ok(agent.clone())
    }

    pub async fn create_capability_profile(
        &self,
        context: &TrustedRequestContext,
        request: CreateCapabilityProfile,
    ) -> Result<CapabilityProfile, ApplicationError> {
        context.require(CAPABILITIES_MANAGE)?;
        request.validate()?;
        let compiled = compile(
            self.catalog.as_ref(),
            context.authority.tenant_id(),
            &request.mappings,
        )
        .await?;
        let profile = CapabilityProfile {
            id: new_profile_id()?,
            tenant_id: context.authority.tenant_id().clone(),
            name: request.name,
            mappings: request.mappings,
            compiled,
            created_by: context.authority.authority().clone(),
            created_at_ms: context.received_at_ms,
        };
        let mut state = self.lock_state()?;
        state
            .tenants
            .entry(context.authority.tenant_id().clone())
            .or_default()
            .profiles
            .insert(profile.id.clone(), profile.clone());
        Ok(profile)
    }

    pub fn list_capability_profiles(
        &self,
        context: &TrustedRequestContext,
    ) -> Result<Vec<CapabilityProfile>, ApplicationError> {
        context.require(CAPABILITIES_READ)?;
        let state = self.lock_state()?;
        Ok(state
            .tenants
            .get(context.authority.tenant_id())
            .map_or_else(Vec::new, |tenant| {
                tenant.profiles.values().cloned().collect()
            }))
    }

    pub fn submit_task(
        &self,
        context: &TrustedRequestContext,
        request: SubmitTask,
    ) -> Result<Task, ApplicationError> {
        context.require(TASKS_SUBMIT)?;
        request.validate()?;
        let mut state = self.lock_state()?;
        let tenant = tenant_mut(&mut state, context);
        if let Some(task_id) = tenant.task_keys.get(&request.idempotency_key)
            && let Some(task) = tenant.tasks.get(task_id)
        {
            if task.agent_id == request.agent_id && task.input == request.input {
                return Ok(task.clone());
            }
            return Err(ApplicationError::IdempotencyConflict);
        }
        let agent = tenant
            .agents
            .get(&request.agent_id)
            .ok_or(ApplicationError::AgentNotFound)?;
        let active_revision = agent
            .active_revision
            .ok_or(ApplicationError::NoActiveRevision)?;
        let revision = tenant
            .revisions
            .get(&request.agent_id)
            .and_then(|revisions| revisions.get(&active_revision))
            .ok_or(ApplicationError::RevisionNotFound)?;
        let task = Task {
            id: new_task_id()?,
            tenant_id: context.authority.tenant_id().clone(),
            agent_id: request.agent_id,
            agent_revision: active_revision,
            capability_profile_id: revision.spec.capability_profile_id.clone(),
            idempotency_key: request.idempotency_key,
            input: request.input,
            status: TaskStatus::Accepted,
            actor: context.authority.authority().clone(),
            executor: context.authority.executor().cloned(),
            delegation_id: context.authority.delegation_id().cloned(),
            request_id: context.request_id.clone(),
            accepted_at_ms: context.received_at_ms,
        };
        tenant
            .task_keys
            .insert(task.idempotency_key.clone(), task.id.clone());
        tenant.tasks.insert(task.id.clone(), task.clone());
        Ok(task)
    }

    pub fn list_tasks(
        &self,
        context: &TrustedRequestContext,
    ) -> Result<Vec<Task>, ApplicationError> {
        context.require(TASKS_READ)?;
        let state = self.lock_state()?;
        Ok(state
            .tenants
            .get(context.authority.tenant_id())
            .map_or_else(Vec::new, |tenant| tenant.tasks.values().cloned().collect()))
    }

    pub fn get_task(
        &self,
        context: &TrustedRequestContext,
        task_id: &TaskId,
    ) -> Result<Task, ApplicationError> {
        context.require(TASKS_READ)?;
        let state = self.lock_state()?;
        tenant(&state, context)
            .and_then(|tenant| tenant.tasks.get(task_id))
            .cloned()
            .ok_or(ApplicationError::TaskNotFound)
    }

    pub fn create_trigger(
        &self,
        context: &TrustedRequestContext,
        request: CreateTrigger,
    ) -> Result<Trigger, ApplicationError> {
        context.require(TRIGGERS_MANAGE)?;
        request.validate()?;
        let mut state = self.lock_state()?;
        let tenant = tenant_mut(&mut state, context);
        let agent = tenant
            .agents
            .get(&request.agent_id)
            .ok_or(ApplicationError::AgentNotFound)?;
        let active_revision = agent
            .active_revision
            .ok_or(ApplicationError::NoActiveRevision)?;
        let trigger = Trigger {
            id: new_trigger_id()?,
            tenant_id: context.authority.tenant_id().clone(),
            name: request.name,
            agent_id: request.agent_id,
            agent_revision: active_revision,
            enabled: request.enabled,
            task_input: request.task_input,
            trigger: request.trigger,
            authority_subject: context.authority.authority().clone(),
            delegation_id: context.authority.delegation_id().cloned(),
            created_at_ms: context.received_at_ms,
        };
        tenant.triggers.insert(trigger.id.clone(), trigger.clone());
        Ok(trigger)
    }

    pub fn list_triggers(
        &self,
        context: &TrustedRequestContext,
    ) -> Result<Vec<Trigger>, ApplicationError> {
        context.require(TRIGGERS_READ)?;
        let state = self.lock_state()?;
        Ok(state
            .tenants
            .get(context.authority.tenant_id())
            .map_or_else(Vec::new, |tenant| {
                tenant.triggers.values().cloned().collect()
            }))
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, State>, ApplicationError> {
        self.state
            .lock()
            .map_err(|_| ApplicationError::StateUnavailable)
    }
}

fn tenant<'a>(state: &'a State, context: &TrustedRequestContext) -> Option<&'a TenantState> {
    state.tenants.get(context.authority.tenant_id())
}

fn tenant_mut<'a>(state: &'a mut State, context: &TrustedRequestContext) -> &'a mut TenantState {
    state
        .tenants
        .entry(context.authority.tenant_id().clone())
        .or_default()
}

fn new_agent_id() -> Result<AgentId, ApplicationError> {
    AgentId::new(format!("agt_{}", Uuid::now_v7().simple())).map_err(Into::into)
}

fn new_profile_id() -> Result<CapabilityProfileId, ApplicationError> {
    CapabilityProfileId::new(format!("cap_{}", Uuid::now_v7().simple())).map_err(Into::into)
}

fn new_task_id() -> Result<TaskId, ApplicationError> {
    TaskId::new(format!("tsk_{}", Uuid::now_v7().simple())).map_err(Into::into)
}

fn new_trigger_id() -> Result<TriggerId, ApplicationError> {
    TriggerId::new(format!("trg_{}", Uuid::now_v7().simple())).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_platform_auth::{
        AGENTS_MANAGE, AGENTS_READ, CAPABILITIES_MANAGE, CAPABILITIES_READ, TASKS_READ,
        TASKS_SUBMIT, TRIGGERS_MANAGE, TRIGGERS_READ,
    };
    use agent_platform_connectors::EmptyCatalog;
    use agent_platform_core::{SubjectId, TenantId};

    fn context(tenant: &str, at: u64) -> TrustedRequestContext {
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
        TrustedRequestContext::new(
            VerifiedAuthority::new(
                TenantId::new(tenant).unwrap(),
                SubjectId::new("human-alice").unwrap(),
                None,
                None,
                scopes,
            )
            .unwrap(),
            RequestId::new(format!("request-{at}")).unwrap(),
            at,
        )
    }

    fn revision() -> RevisionSpec {
        RevisionSpec {
            instructions: "Help the user.".to_owned(),
            model: "model-one".to_owned(),
            capability_profile_id: None,
            metadata: None,
        }
    }

    fn active_agent(app: &Application, context: &TrustedRequestContext) -> (Agent, AgentRevision) {
        let agent = app
            .create_agent(
                context,
                CreateAgent {
                    name: "Helper".to_owned(),
                },
            )
            .unwrap();
        let revision = app.create_revision(context, &agent.id, revision()).unwrap();
        app.activate_revision(
            context,
            &agent.id,
            &ActivateRevision {
                revision: revision.revision,
                expected_active_revision: None,
            },
        )
        .unwrap();
        (agent, revision)
    }

    #[test]
    fn tenant_partition_is_selected_before_agent_lookup() {
        let app = Application::new(Arc::new(EmptyCatalog));
        let tenant_one = context("tenant-one", 1);
        let tenant_two = context("tenant-two", 2);
        let agent = app
            .create_agent(
                &tenant_one,
                CreateAgent {
                    name: "Private".to_owned(),
                },
            )
            .unwrap();
        assert_eq!(
            app.get_agent(&tenant_two, &agent.id),
            Err(ApplicationError::AgentNotFound)
        );
        assert!(app.list_agents(&tenant_two).unwrap().is_empty());
    }

    #[test]
    fn activation_is_compare_and_swap_and_revisions_do_not_change() {
        let app = Application::new(Arc::new(EmptyCatalog));
        let context = context("tenant-one", 1);
        let (agent, first) = active_agent(&app, &context);
        let second = app
            .create_revision(&context, &agent.id, revision())
            .unwrap();
        let error = app
            .activate_revision(
                &context,
                &agent.id,
                &ActivateRevision {
                    revision: second.revision,
                    expected_active_revision: None,
                },
            )
            .unwrap_err();
        assert_eq!(
            error,
            ApplicationError::ActiveRevisionConflict {
                expected: None,
                actual: Some(first.revision)
            }
        );
        assert_eq!(app.list_revisions(&context, &agent.id).unwrap()[0], first);
    }

    #[test]
    fn equal_task_retry_returns_original_but_changed_intent_conflicts() {
        let app = Application::new(Arc::new(EmptyCatalog));
        let context = context("tenant-one", 10);
        let (agent, revision) = active_agent(&app, &context);
        let request = SubmitTask {
            agent_id: agent.id.clone(),
            idempotency_key: "retry-one".to_owned(),
            input: serde_json::json!({"prompt": "hello"}),
        };
        let first = app.submit_task(&context, request.clone()).unwrap();
        let second = app.submit_task(&context, request).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.agent_revision, revision.revision);

        let changed = app.submit_task(
            &context,
            SubmitTask {
                agent_id: agent.id,
                idempotency_key: "retry-one".to_owned(),
                input: serde_json::json!({"prompt": "different"}),
            },
        );
        assert_eq!(changed, Err(ApplicationError::IdempotencyConflict));
    }
}
