#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agent_platform_auth::{
    AGENTS_MANAGE, AGENTS_READ, CAPABILITIES_MANAGE, CAPABILITIES_READ, TASKS_READ, TASKS_SUBMIT,
    TRIGGERS_MANAGE, TRIGGERS_READ, VerifiedAuthority,
};
use agent_platform_connectors::{
    CompiledToolset, ConnectorCatalog, InMemoryCatalog, ProjectionError, compile,
};
use agent_platform_core::{
    ActivateRevision, Agent, AgentId, AgentRevision, AttemptId, CapabilityProfileId, CreateAgent,
    CreateCapabilityProfile, CreateTrigger, PendingApproval, RequestId, RevisionSpec, SubmitTask,
    Task, TaskEvent, TaskEventKind, TaskFailure, TaskId, TaskStatus, TenantId, Trigger, TriggerId,
    UpdateCapabilityProfile, ValidationError,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CapabilityProfile {
    pub id: CapabilityProfileId,
    pub tenant_id: TenantId,
    pub name: String,
    #[serde(default = "initial_profile_revision")]
    pub revision: u64,
    pub mappings: Vec<agent_platform_core::CapabilityMapping>,
    pub compiled: CompiledToolset,
    pub created_by: agent_platform_core::SubjectId,
    pub created_at_ms: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone)]
pub struct TaskExecutionPlan {
    pub task: Task,
    pub revision: RevisionSpec,
    pub toolset: Option<CompiledToolset>,
}

#[derive(Debug, Clone)]
pub struct TaskAdmission {
    pub plan: TaskExecutionPlan,
    pub newly_created: bool,
}

pub struct TaskEventSubscription {
    pub backlog: Vec<TaskEvent>,
    pub receiver: broadcast::Receiver<TaskEvent>,
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
    #[error("capability profile revision changed; expected {expected}, found {actual}")]
    CapabilityProfileRevisionConflict { expected: u64, actual: u64 },
    #[error("idempotency key was already used for different task intent")]
    IdempotencyConflict,
    #[error("application state lock is unavailable")]
    StateUnavailable,
    #[error("durable application state is unavailable")]
    StatePersistence,
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    #[error(transparent)]
    Projection(#[from] ProjectionError),
}

const MAX_STATE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    tenants: BTreeMap<TenantId, TenantState>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TenantState {
    agents: BTreeMap<AgentId, Agent>,
    revisions: BTreeMap<AgentId, BTreeMap<u64, AgentRevision>>,
    profiles: BTreeMap<CapabilityProfileId, CapabilityProfile>,
    tasks: BTreeMap<TaskId, Task>,
    task_keys: BTreeMap<String, TaskId>,
    task_events: BTreeMap<TaskId, Vec<TaskEvent>>,
    #[serde(skip)]
    task_event_senders: BTreeMap<TaskId, broadcast::Sender<TaskEvent>>,
    triggers: BTreeMap<TriggerId, Trigger>,
}

#[derive(Clone)]
pub struct Application {
    state: Arc<Mutex<State>>,
    catalog: Arc<dyn ConnectorCatalog>,
    state_path: Option<Arc<PathBuf>>,
}

impl std::fmt::Debug for Application {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Application")
            .field("state", &"tenant-scoped")
            .field("catalog", &"ConnectorCatalog")
            .field("durable", &self.state_path.is_some())
            .finish()
    }
}

impl Application {
    pub fn new(catalog: Arc<dyn ConnectorCatalog>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            catalog,
            state_path: None,
        }
    }

    /// Open credential-free durable state and recover interrupted attempts as failed evidence.
    pub fn open(
        catalog: Arc<dyn ConnectorCatalog>,
        path: impl Into<PathBuf>,
        recovered_at_ms: u64,
    ) -> Result<Self, ApplicationError> {
        let path = path.into();
        let mut state = read_state(&path)?;
        rebuild_task_senders(&mut state);
        recover_interrupted_tasks(&mut state, recovered_at_ms)?;
        persist_state(&state, &path)?;
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            catalog,
            state_path: Some(Arc::new(path)),
        })
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
        self.persist(&state)?;
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
        self.persist(&state)?;
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
        let agent = agent.clone();
        self.persist(&state)?;
        Ok(agent)
    }

    pub async fn create_capability_profile(
        &self,
        context: &TrustedRequestContext,
        request: CreateCapabilityProfile,
    ) -> Result<CapabilityProfile, ApplicationError> {
        context.require(CAPABILITIES_MANAGE)?;
        request.validate()?;
        let request_catalog = InMemoryCatalog::new(request.operation_descriptions.clone());
        let catalog: &dyn ConnectorCatalog = if request.operation_descriptions.is_empty() {
            self.catalog.as_ref()
        } else {
            &request_catalog
        };
        let compiled = compile(catalog, context.authority.tenant_id(), &request.mappings).await?;
        let profile = CapabilityProfile {
            id: new_profile_id()?,
            tenant_id: context.authority.tenant_id().clone(),
            name: request.name,
            revision: 1,
            mappings: request.mappings,
            compiled,
            created_by: context.authority.authority().clone(),
            created_at_ms: context.received_at_ms,
            updated_at_ms: context.received_at_ms,
        };
        let mut state = self.lock_state()?;
        state
            .tenants
            .entry(context.authority.tenant_id().clone())
            .or_default()
            .profiles
            .insert(profile.id.clone(), profile.clone());
        self.persist(&state)?;
        Ok(profile)
    }

    pub async fn update_capability_profile(
        &self,
        context: &TrustedRequestContext,
        profile_id: &CapabilityProfileId,
        request: UpdateCapabilityProfile,
    ) -> Result<CapabilityProfile, ApplicationError> {
        context.require(CAPABILITIES_MANAGE)?;
        request.validate()?;
        let request_catalog = InMemoryCatalog::new(request.operation_descriptions.clone());
        let catalog: &dyn ConnectorCatalog = if request.operation_descriptions.is_empty() {
            self.catalog.as_ref()
        } else {
            &request_catalog
        };
        let compiled = compile(catalog, context.authority.tenant_id(), &request.mappings).await?;
        let mut state = self.lock_state()?;
        let tenant = tenant_mut(&mut state, context);
        let profile = tenant
            .profiles
            .get_mut(profile_id)
            .ok_or(ApplicationError::CapabilityProfileNotFound)?;
        if profile.revision != request.expected_revision {
            return Err(ApplicationError::CapabilityProfileRevisionConflict {
                expected: request.expected_revision,
                actual: profile.revision,
            });
        }
        profile.revision = profile
            .revision
            .checked_add(1)
            .ok_or(ApplicationError::StateUnavailable)?;
        profile.name = request.name;
        profile.mappings = request.mappings;
        profile.compiled = compiled;
        profile.updated_at_ms = context.received_at_ms;
        let profile = profile.clone();
        self.persist(&state)?;
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
        let attempt_id = new_attempt_id()?;
        Ok(self.admit_task(context, request, attempt_id)?.plan.task)
    }

    pub fn admit_task(
        &self,
        context: &TrustedRequestContext,
        request: SubmitTask,
        attempt_id: AttemptId,
    ) -> Result<TaskAdmission, ApplicationError> {
        context.require(TASKS_SUBMIT)?;
        request.validate()?;
        let mut state = self.lock_state()?;
        let tenant = tenant_mut(&mut state, context);
        if let Some(task_id) = tenant.task_keys.get(&request.idempotency_key)
            && let Some(task) = tenant.tasks.get(task_id)
        {
            if task.agent_id == request.agent_id && task.input == request.input {
                let revision = tenant
                    .revisions
                    .get(&task.agent_id)
                    .and_then(|revisions| revisions.get(&task.agent_revision))
                    .ok_or(ApplicationError::RevisionNotFound)?;
                let toolset = task
                    .capability_profile_id
                    .as_ref()
                    .and_then(|id| tenant.profiles.get(id))
                    .map(|profile| profile.compiled.clone());
                return Ok(TaskAdmission {
                    plan: TaskExecutionPlan {
                        task: task.clone(),
                        revision: revision.spec.clone(),
                        toolset,
                    },
                    newly_created: false,
                });
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
            attempt_id: attempt_id.clone(),
            output: None,
            failure: None,
            actor: context.authority.authority().clone(),
            executor: context.authority.executor().cloned(),
            delegation_id: context.authority.delegation_id().cloned(),
            request_id: context.request_id.clone(),
            accepted_at_ms: context.received_at_ms,
            completed_at_ms: None,
        };
        let toolset = task
            .capability_profile_id
            .as_ref()
            .and_then(|id| tenant.profiles.get(id))
            .map(|profile| profile.compiled.clone());
        let revision = revision.spec.clone();
        tenant
            .task_keys
            .insert(task.idempotency_key.clone(), task.id.clone());
        tenant.tasks.insert(task.id.clone(), task.clone());
        let event = TaskEvent {
            task_id: task.id.clone(),
            attempt_id,
            sequence: 1,
            occurred_at_ms: context.received_at_ms,
            event: TaskEventKind::Accepted,
        };
        tenant
            .task_events
            .insert(task.id.clone(), vec![event.clone()]);
        let (sender, _) = broadcast::channel(256);
        let _ = sender.send(event);
        tenant.task_event_senders.insert(task.id.clone(), sender);
        self.persist(&state)?;
        Ok(TaskAdmission {
            plan: TaskExecutionPlan {
                task,
                revision,
                toolset,
            },
            newly_created: true,
        })
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

    pub fn get_task_for_approval(
        &self,
        context: &TrustedRequestContext,
        task_id: &TaskId,
    ) -> Result<Task, ApplicationError> {
        context.require(TASKS_SUBMIT)?;
        let state = self.lock_state()?;
        tenant(&state, context)
            .and_then(|tenant| tenant.tasks.get(task_id))
            .cloned()
            .ok_or(ApplicationError::TaskNotFound)
    }

    pub fn subscribe_task_events(
        &self,
        context: &TrustedRequestContext,
        task_id: &TaskId,
    ) -> Result<TaskEventSubscription, ApplicationError> {
        context.require(TASKS_READ)?;
        let state = self.lock_state()?;
        let tenant = tenant(&state, context).ok_or(ApplicationError::TaskNotFound)?;
        if !tenant.tasks.contains_key(task_id) {
            return Err(ApplicationError::TaskNotFound);
        }
        let backlog = tenant.task_events.get(task_id).cloned().unwrap_or_default();
        let receiver = tenant
            .task_event_senders
            .get(task_id)
            .ok_or(ApplicationError::StateUnavailable)?
            .subscribe();
        Ok(TaskEventSubscription { backlog, receiver })
    }

    pub fn task_events_after(
        &self,
        context: &TrustedRequestContext,
        task_id: &TaskId,
        sequence: u64,
    ) -> Result<Vec<TaskEvent>, ApplicationError> {
        context.require(TASKS_READ)?;
        let state = self.lock_state()?;
        let tenant = tenant(&state, context).ok_or(ApplicationError::TaskNotFound)?;
        if !tenant.tasks.contains_key(task_id) {
            return Err(ApplicationError::TaskNotFound);
        }
        Ok(tenant
            .task_events
            .get(task_id)
            .map_or_else(Vec::new, |events| {
                events
                    .iter()
                    .filter(|event| event.sequence > sequence)
                    .cloned()
                    .collect()
            }))
    }

    pub fn mark_task_running(
        &self,
        tenant_id: &TenantId,
        task_id: &TaskId,
        attempt_id: &AttemptId,
        at_ms: u64,
    ) -> Result<(), ApplicationError> {
        self.transition_task(
            tenant_id,
            task_id,
            attempt_id,
            at_ms,
            TaskStatus::Running,
            TaskEventKind::Running,
            None,
            None,
        )
    }

    pub fn mark_task_awaiting_approval(
        &self,
        tenant_id: &TenantId,
        approval: &PendingApproval,
    ) -> Result<(), ApplicationError> {
        self.transition_task(
            tenant_id,
            &approval.task_id,
            &approval.attempt_id,
            approval.requested_at_ms,
            TaskStatus::AwaitingApproval,
            TaskEventKind::ApprovalRequested {
                approval_id: approval.id.clone(),
                call_id: approval.call_id.clone(),
                operation_ref: approval.operation_ref.clone(),
                connection_ref: approval.connection_ref.clone(),
            },
            None,
            None,
        )
    }

    pub fn resolve_task_approval(
        &self,
        tenant_id: &TenantId,
        task_id: &TaskId,
        attempt_id: &AttemptId,
        at_ms: u64,
        approval_id: agent_platform_core::ApprovalId,
        approved: bool,
    ) -> Result<(), ApplicationError> {
        self.transition_task(
            tenant_id,
            task_id,
            attempt_id,
            at_ms,
            TaskStatus::Running,
            TaskEventKind::ApprovalResolved {
                approval_id,
                approved,
            },
            None,
            None,
        )
    }

    pub fn append_task_text(
        &self,
        tenant_id: &TenantId,
        task_id: &TaskId,
        attempt_id: &AttemptId,
        at_ms: u64,
        text: String,
    ) -> Result<(), ApplicationError> {
        let mut state = self.lock_state()?;
        let tenant = state
            .tenants
            .get_mut(tenant_id)
            .ok_or(ApplicationError::TaskNotFound)?;
        require_attempt(tenant, task_id, attempt_id)?;
        append_event(
            tenant,
            task_id,
            attempt_id,
            at_ms,
            TaskEventKind::TextDelta { text },
        )?;
        self.persist(&state)
    }

    pub fn succeed_task(
        &self,
        tenant_id: &TenantId,
        task_id: &TaskId,
        attempt_id: &AttemptId,
        at_ms: u64,
        output: String,
    ) -> Result<(), ApplicationError> {
        self.transition_task(
            tenant_id,
            task_id,
            attempt_id,
            at_ms,
            TaskStatus::Succeeded,
            TaskEventKind::Succeeded {
                output: output.clone(),
            },
            Some(output),
            None,
        )
    }

    pub fn fail_task(
        &self,
        tenant_id: &TenantId,
        task_id: &TaskId,
        attempt_id: &AttemptId,
        at_ms: u64,
        failure: TaskFailure,
    ) -> Result<(), ApplicationError> {
        self.transition_task(
            tenant_id,
            task_id,
            attempt_id,
            at_ms,
            TaskStatus::Failed,
            TaskEventKind::Failed {
                failure: failure.clone(),
            },
            None,
            Some(failure),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn transition_task(
        &self,
        tenant_id: &TenantId,
        task_id: &TaskId,
        attempt_id: &AttemptId,
        at_ms: u64,
        status: TaskStatus,
        event: TaskEventKind,
        output: Option<String>,
        failure: Option<TaskFailure>,
    ) -> Result<(), ApplicationError> {
        let mut state = self.lock_state()?;
        let tenant = state
            .tenants
            .get_mut(tenant_id)
            .ok_or(ApplicationError::TaskNotFound)?;
        let task = tenant
            .tasks
            .get_mut(task_id)
            .ok_or(ApplicationError::TaskNotFound)?;
        if &task.attempt_id != attempt_id {
            return Err(ApplicationError::TaskNotFound);
        }
        task.status = status;
        task.output = output;
        task.failure = failure;
        if matches!(status, TaskStatus::Succeeded | TaskStatus::Failed) {
            task.completed_at_ms = Some(at_ms);
        }
        append_event(tenant, task_id, attempt_id, at_ms, event)?;
        self.persist(&state)
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
        self.persist(&state)?;
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

    fn persist(&self, state: &State) -> Result<(), ApplicationError> {
        self.state_path
            .as_deref()
            .map_or(Ok(()), |path| persist_state(state, path))
    }
}

fn read_state(path: &Path) -> Result<State, ApplicationError> {
    if !path.exists() {
        return Ok(State::default());
    }
    let metadata = std::fs::metadata(path).map_err(|_| ApplicationError::StatePersistence)?;
    if !metadata.is_file() || metadata.len() > MAX_STATE_BYTES {
        return Err(ApplicationError::StatePersistence);
    }
    let bytes = std::fs::read(path).map_err(|_| ApplicationError::StatePersistence)?;
    serde_json::from_slice(&bytes).map_err(|_| ApplicationError::StatePersistence)
}

fn persist_state(state: &State, path: &Path) -> Result<(), ApplicationError> {
    let bytes = serde_json::to_vec(state).map_err(|_| ApplicationError::StatePersistence)?;
    if bytes.len() as u64 > MAX_STATE_BYTES {
        return Err(ApplicationError::StatePersistence);
    }
    let parent = path.parent().ok_or(ApplicationError::StatePersistence)?;
    std::fs::create_dir_all(parent).map_err(|_| ApplicationError::StatePersistence)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(ApplicationError::StatePersistence)?;
    let temporary = parent.join(format!(".{file_name}.new"));
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| ApplicationError::StatePersistence)?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| ApplicationError::StatePersistence)?;
    std::fs::rename(&temporary, path).map_err(|_| ApplicationError::StatePersistence)
}

fn rebuild_task_senders(state: &mut State) {
    for tenant in state.tenants.values_mut() {
        for task_id in tenant.tasks.keys() {
            let (sender, _) = broadcast::channel(256);
            tenant.task_event_senders.insert(task_id.clone(), sender);
        }
    }
}

fn recover_interrupted_tasks(
    state: &mut State,
    recovered_at_ms: u64,
) -> Result<(), ApplicationError> {
    for tenant in state.tenants.values_mut() {
        let interrupted = tenant
            .tasks
            .values()
            .filter(|task| {
                matches!(
                    task.status,
                    TaskStatus::Accepted | TaskStatus::Running | TaskStatus::AwaitingApproval
                )
            })
            .map(|task| (task.id.clone(), task.attempt_id.clone()))
            .collect::<Vec<_>>();
        for (task_id, attempt_id) in interrupted {
            let failure = TaskFailure {
                code: "execution_interrupted".to_owned(),
                message: "the service restarted before the attempt reached a terminal state"
                    .to_owned(),
            };
            let task = tenant
                .tasks
                .get_mut(&task_id)
                .ok_or(ApplicationError::TaskNotFound)?;
            task.status = TaskStatus::Failed;
            task.failure = Some(failure.clone());
            task.completed_at_ms = Some(recovered_at_ms);
            append_event(
                tenant,
                &task_id,
                &attempt_id,
                recovered_at_ms,
                TaskEventKind::Failed { failure },
            )?;
        }
    }
    Ok(())
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

const fn initial_profile_revision() -> u64 {
    1
}

fn new_task_id() -> Result<TaskId, ApplicationError> {
    TaskId::new(format!("tsk_{}", Uuid::now_v7().simple())).map_err(Into::into)
}

fn new_attempt_id() -> Result<AttemptId, ApplicationError> {
    AttemptId::new(format!("atm_{}", Uuid::now_v7().simple())).map_err(Into::into)
}

fn require_attempt(
    tenant: &TenantState,
    task_id: &TaskId,
    attempt_id: &AttemptId,
) -> Result<(), ApplicationError> {
    if tenant
        .tasks
        .get(task_id)
        .is_some_and(|task| &task.attempt_id == attempt_id)
    {
        Ok(())
    } else {
        Err(ApplicationError::TaskNotFound)
    }
}

fn append_event(
    tenant: &mut TenantState,
    task_id: &TaskId,
    attempt_id: &AttemptId,
    at_ms: u64,
    event: TaskEventKind,
) -> Result<(), ApplicationError> {
    let events = tenant.task_events.entry(task_id.clone()).or_default();
    let sequence = u64::try_from(events.len())
        .ok()
        .and_then(|value| value.checked_add(1))
        .ok_or(ApplicationError::StateUnavailable)?;
    let event = TaskEvent {
        task_id: task_id.clone(),
        attempt_id: attempt_id.clone(),
        sequence,
        occurred_at_ms: at_ms,
        event,
    };
    events.push(event.clone());
    if let Some(sender) = tenant.task_event_senders.get(task_id) {
        let _ = sender.send(event);
    }
    Ok(())
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
    use agent_platform_connectors::{
        ApprovalPosture, ConnectionSummary, EffectClass, EmptyCatalog, OperationDescription,
    };
    use agent_platform_core::{CapabilityMapping, CapabilityPosture, SubjectId, TenantId};

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

    #[test]
    fn execution_events_are_ordered_and_exact_attempt_tenant_scoped() {
        let app = Application::new(Arc::new(EmptyCatalog));
        let tenant_one = context("tenant-one", 10);
        let other = context("tenant-two", 11);
        let (agent, _) = active_agent(&app, &tenant_one);
        let task = app
            .submit_task(
                &tenant_one,
                SubmitTask {
                    agent_id: agent.id,
                    idempotency_key: "execution-one".to_owned(),
                    input: serde_json::json!({"prompt": "hello"}),
                },
            )
            .unwrap();
        app.mark_task_running(
            &task.tenant_id,
            &task.id,
            &task.attempt_id,
            task.accepted_at_ms + 1,
        )
        .unwrap();
        app.append_task_text(
            &task.tenant_id,
            &task.id,
            &task.attempt_id,
            task.accepted_at_ms + 2,
            "hello".to_owned(),
        )
        .unwrap();
        app.succeed_task(
            &task.tenant_id,
            &task.id,
            &task.attempt_id,
            task.accepted_at_ms + 3,
            "hello".to_owned(),
        )
        .unwrap();

        let events = app
            .subscribe_task_events(&tenant_one, &task.id)
            .unwrap()
            .backlog;
        assert_eq!(
            events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert!(matches!(events[0].event, TaskEventKind::Accepted));
        assert!(matches!(events[1].event, TaskEventKind::Running));
        assert!(matches!(events[2].event, TaskEventKind::TextDelta { .. }));
        assert!(matches!(events[3].event, TaskEventKind::Succeeded { .. }));
        assert!(matches!(
            app.subscribe_task_events(&other, &task.id),
            Err(ApplicationError::TaskNotFound)
        ));
        let wrong_attempt = AttemptId::new("atm_wrong").unwrap();
        assert_eq!(
            app.mark_task_running(&task.tenant_id, &task.id, &wrong_attempt, 99),
            Err(ApplicationError::TaskNotFound)
        );
    }

    #[test]
    fn durable_state_survives_restart_and_closes_interrupted_attempts() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("agent-platform.json");
        let context = context("tenant-one", 10);
        let app = Application::open(Arc::new(EmptyCatalog), &path, 10).unwrap();
        let (created_agent, _) = active_agent(&app, &context);
        let agent = app.get_agent(&context, &created_agent.id).unwrap();
        let task = app
            .submit_task(
                &context,
                SubmitTask {
                    agent_id: agent.id.clone(),
                    idempotency_key: "durable-task".to_owned(),
                    input: serde_json::json!({"prompt": "hello"}),
                },
            )
            .unwrap();
        app.mark_task_running(&task.tenant_id, &task.id, &task.attempt_id, 11)
            .unwrap();
        drop(app);

        let reopened = Application::open(Arc::new(EmptyCatalog), &path, 20).unwrap();
        assert_eq!(reopened.list_agents(&context).unwrap(), vec![agent]);
        let recovered = reopened.get_task(&context, &task.id).unwrap();
        assert_eq!(recovered.status, TaskStatus::Failed);
        assert_eq!(recovered.failure.unwrap().code, "execution_interrupted");
        assert_eq!(
            reopened
                .subscribe_task_events(&context, &task.id)
                .unwrap()
                .backlog
                .last()
                .unwrap()
                .sequence,
            3
        );
    }

    #[tokio::test]
    async fn capability_profile_updates_are_compare_and_swap_and_change_posture() {
        let description = OperationDescription {
            operation_ref: "repository.read".to_owned(),
            title: "Read repository".to_owned(),
            description: "Read one repository file.".to_owned(),
            input_schema: serde_json::json!({"type": "object"}),
            output_schema: serde_json::json!({"type": "object"}),
            effect: EffectClass::ReadOnly,
            approval: ApprovalPosture::NotRequired,
            connections: vec![ConnectionSummary {
                connection_ref: "connection-project".to_owned(),
                label: "Project".to_owned(),
                provider: "workspace".to_owned(),
                audiences: Vec::new(),
                purpose: None,
            }],
            description_ref: "description-project-read".to_owned(),
        };
        let app = Application::new(Arc::new(EmptyCatalog));
        let context = context("tenant-one", 10);
        let mapping = |posture| CapabilityMapping {
            operation_ref: "repository.read".to_owned(),
            tool_name: "repository_read".to_owned(),
            connection_ref: Some("connection-project".to_owned()),
            context: None,
            posture,
        };
        let profile = app
            .create_capability_profile(
                &context,
                CreateCapabilityProfile {
                    name: "Project".to_owned(),
                    mappings: vec![mapping(CapabilityPosture::Allow)],
                    operation_descriptions: vec![description.clone()],
                },
            )
            .await
            .unwrap();
        assert_eq!(profile.revision, 1);
        assert_eq!(profile.compiled.capabilities.len(), 1);

        let updated = app
            .update_capability_profile(
                &context,
                &profile.id,
                UpdateCapabilityProfile {
                    expected_revision: 1,
                    name: "Project".to_owned(),
                    mappings: vec![mapping(CapabilityPosture::Deny)],
                    operation_descriptions: vec![description.clone()],
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.revision, 2);
        assert!(updated.compiled.capabilities.is_empty());
        assert!(matches!(
            app.update_capability_profile(
                &context,
                &profile.id,
                UpdateCapabilityProfile {
                    expected_revision: 1,
                    name: "Stale".to_owned(),
                    mappings: vec![mapping(CapabilityPosture::Allow)],
                    operation_descriptions: vec![description],
                },
            )
            .await,
            Err(ApplicationError::CapabilityProfileRevisionConflict {
                expected: 1,
                actual: 2
            })
        ));
    }
}
