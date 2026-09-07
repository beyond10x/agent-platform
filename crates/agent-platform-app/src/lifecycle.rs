use super::{
    Application, ApplicationError, State, TenantState, TrustedRequestContext, agent_owned_by,
    require_visible_profile, task_owned_by, tenant, tenant_mut,
};
use agent_platform_auth::{AGENTS_MANAGE, CAPABILITIES_MANAGE, TASKS_READ, TASKS_SUBMIT};
use agent_platform_core::{
    Agent, AgentId, AgentRevision, CapabilityProfileId, Conversation, ConversationId, CreateAgent,
    SubmitTask, Task, TaskStatus, ValidationError,
};
use agent_platform_core::{
    ConversationInput, ConversationMessage, ConversationRole, CreateConversation, UpdateAgent,
    UpdateConversation,
};
use std::collections::BTreeSet;
use uuid::Uuid;

pub(super) fn require_active_agent<'a>(
    tenant: &'a TenantState,
    context: &TrustedRequestContext,
    id: &AgentId,
) -> Result<&'a Agent, ApplicationError> {
    tenant
        .agents
        .get(id)
        .filter(|agent| agent_owned_by(agent, context) && !tenant.retired_agents.contains_key(id))
        .ok_or(ApplicationError::AgentNotFound)
}

fn active(task: &Task) -> bool {
    matches!(
        task.status,
        TaskStatus::Accepted | TaskStatus::Running | TaskStatus::AwaitingApproval
    )
}

fn conversation<'a>(
    tenant: &'a TenantState,
    context: &TrustedRequestContext,
    agent_id: &AgentId,
    id: &ConversationId,
) -> Result<&'a Conversation, ApplicationError> {
    require_active_agent(tenant, context, agent_id)?;
    tenant
        .conversations
        .get(id)
        .filter(|item| {
            &item.agent_id == agent_id
                && item.deleted_at_ms.is_none()
                && &item.created_by == context.authority.authority()
        })
        .ok_or(ApplicationError::ConversationNotFound)
}

fn check_conversation_mutation(
    tenant: &TenantState,
    item: &Conversation,
    expected_revision: u64,
) -> Result<(), ApplicationError> {
    if item.revision != expected_revision {
        return Err(ApplicationError::ConversationRevisionConflict);
    }
    if item
        .task_ids
        .iter()
        .filter_map(|id| tenant.tasks.get(id))
        .any(active)
    {
        return Err(ApplicationError::ActiveWork);
    }
    Ok(())
}

fn new_conversation(
    agent: &Agent,
    title: String,
    at_ms: u64,
) -> Result<Conversation, ApplicationError> {
    Ok(Conversation {
        id: ConversationId::new(format!("con_{}", Uuid::now_v7().simple()))?,
        agent_id: agent.id.clone(),
        created_by: agent.created_by.clone(),
        title,
        revision: 1,
        created_at_ms: at_ms,
        deleted_at_ms: None,
        task_ids: Vec::new(),
    })
}

impl Application {
    // Keep a refused mutation and a failed durable write from changing the live state.
    fn lifecycle_change<T>(
        &self,
        change: impl FnOnce(&mut State) -> Result<T, ApplicationError>,
    ) -> Result<T, ApplicationError> {
        let mut state = self.lock_state()?;
        let mut candidate = state.clone();
        let result = change(&mut candidate)?;
        self.persist(&candidate)?;
        *state = candidate;
        Ok(result)
    }

    pub fn update_agent(
        &self,
        context: &TrustedRequestContext,
        id: &AgentId,
        request: UpdateAgent,
    ) -> Result<Agent, ApplicationError> {
        context.require(AGENTS_MANAGE)?;
        CreateAgent {
            name: request.name.clone(),
        }
        .validate()?;
        request.spec.validate()?;
        self.lifecycle_change(|state| {
            let tenant = tenant_mut(state, context);
            let agent = require_active_agent(tenant, context, id)?;
            if agent.active_revision != request.expected_active_revision {
                return Err(ApplicationError::ActiveRevisionConflict {
                    expected: request.expected_active_revision,
                    actual: agent.active_revision,
                });
            }
            require_visible_profile(tenant, request.spec.capability_profile_id.as_ref(), context)?;
            let number = agent
                .latest_revision
                .checked_add(1)
                .ok_or(ApplicationError::StateUnavailable)?;
            let mut agent = agent.clone();
            agent.name = request.name;
            agent.latest_revision = number;
            agent.active_revision = Some(number);
            let revision = AgentRevision {
                agent_id: id.clone(),
                tenant_id: agent.tenant_id.clone(),
                revision: number,
                spec: request.spec,
                created_by: context.authority.authority().clone(),
                created_at_ms: context.received_at_ms,
            };
            tenant
                .revisions
                .entry(id.clone())
                .or_default()
                .insert(number, revision);
            tenant.agents.insert(id.clone(), agent.clone());
            Ok(agent)
        })
    }

    pub fn retire_agent(
        &self,
        context: &TrustedRequestContext,
        id: &AgentId,
    ) -> Result<(), ApplicationError> {
        context.require(AGENTS_MANAGE)?;
        self.lifecycle_change(|state| {
            let tenant = tenant_mut(state, context);
            require_active_agent(tenant, context, id)?;
            if tenant
                .tasks
                .values()
                .any(|task| &task.agent_id == id && active(task))
            {
                return Err(ApplicationError::ActiveWork);
            }
            tenant
                .retired_agents
                .insert(id.clone(), context.received_at_ms);
            for trigger in tenant
                .triggers
                .values_mut()
                .filter(|trigger| &trigger.agent_id == id)
            {
                trigger.enabled = false;
            }
            Ok(())
        })
    }

    pub fn retire_capability_profile(
        &self,
        context: &TrustedRequestContext,
        id: &CapabilityProfileId,
    ) -> Result<(), ApplicationError> {
        context.require(CAPABILITIES_MANAGE)?;
        self.lifecycle_change(|state| {
            let tenant = tenant_mut(state, context);
            require_visible_profile(tenant, Some(id), context)?;
            if tenant.profiles[id].created_by != *context.authority.authority() {
                return Err(ApplicationError::CapabilityProfileNotFound);
            }
            if tenant
                .agents
                .values()
                .filter(|agent| !tenant.retired_agents.contains_key(&agent.id))
                .filter_map(|agent| {
                    agent
                        .active_revision
                        .and_then(|number| tenant.revisions.get(&agent.id)?.get(&number))
                })
                .any(|revision| revision.spec.capability_profile_id.as_ref() == Some(id))
                || tenant
                    .tasks
                    .values()
                    .any(|task| task.capability_profile_id.as_ref() == Some(id) && active(task))
            {
                return Err(ApplicationError::CapabilityProfileInUse);
            }
            tenant
                .retired_profiles
                .insert(id.clone(), context.received_at_ms);
            Ok(())
        })
    }

    pub fn create_conversation(
        &self,
        context: &TrustedRequestContext,
        agent_id: &AgentId,
        request: CreateConversation,
    ) -> Result<Conversation, ApplicationError> {
        context.require(TASKS_SUBMIT)?;
        request.validate()?;
        self.lifecycle_change(|state| {
            let tenant = tenant_mut(state, context);
            let agent = require_active_agent(tenant, context, agent_id)?;
            let item = new_conversation(agent, request.title, context.received_at_ms)?;
            tenant.conversations.insert(item.id.clone(), item.clone());
            Ok(item)
        })
    }

    pub fn list_conversations(
        &self,
        context: &TrustedRequestContext,
        agent_id: &AgentId,
    ) -> Result<Vec<Conversation>, ApplicationError> {
        context.require(TASKS_READ)?;
        let state = self.lock_state()?;
        let tenant = tenant(&state, context).ok_or(ApplicationError::AgentNotFound)?;
        require_active_agent(tenant, context, agent_id)?;
        Ok(tenant
            .conversations
            .values()
            .filter(|item| {
                &item.agent_id == agent_id
                    && item.deleted_at_ms.is_none()
                    && &item.created_by == context.authority.authority()
            })
            .cloned()
            .collect())
    }

    pub fn list_conversation_tasks(
        &self,
        context: &TrustedRequestContext,
        agent_id: &AgentId,
        id: &ConversationId,
    ) -> Result<Vec<Task>, ApplicationError> {
        context.require(TASKS_READ)?;
        let state = self.lock_state()?;
        let tenant = tenant(&state, context).ok_or(ApplicationError::ConversationNotFound)?;
        let item = conversation(tenant, context, agent_id, id)?;
        Ok(item
            .task_ids
            .iter()
            .filter_map(|id| tenant.tasks.get(id))
            .filter(|task| task_owned_by(task, context))
            .cloned()
            .collect())
    }

    pub fn update_conversation(
        &self,
        context: &TrustedRequestContext,
        agent_id: &AgentId,
        id: &ConversationId,
        request: UpdateConversation,
    ) -> Result<Conversation, ApplicationError> {
        context.require(TASKS_SUBMIT)?;
        CreateConversation {
            title: request.title.clone(),
        }
        .validate()?;
        self.lifecycle_change(|state| {
            let tenant = tenant_mut(state, context);
            let item = conversation(tenant, context, agent_id, id)?;
            if item.revision != request.expected_revision {
                return Err(ApplicationError::ConversationRevisionConflict);
            }
            let mut item = item.clone();
            item.title = request.title;
            item.revision = item
                .revision
                .checked_add(1)
                .ok_or(ApplicationError::StateUnavailable)?;
            tenant.conversations.insert(id.clone(), item.clone());
            Ok(item)
        })
    }

    pub fn delete_conversation(
        &self,
        context: &TrustedRequestContext,
        agent_id: &AgentId,
        id: &ConversationId,
        expected_revision: u64,
    ) -> Result<(), ApplicationError> {
        context.require(TASKS_SUBMIT)?;
        self.lifecycle_change(|state| {
            let tenant = tenant_mut(state, context);
            let item = conversation(tenant, context, agent_id, id)?;
            check_conversation_mutation(tenant, item, expected_revision)?;
            tenant
                .conversations
                .get_mut(id)
                .ok_or(ApplicationError::ConversationNotFound)?
                .deleted_at_ms = Some(context.received_at_ms);
            Ok(())
        })
    }

    pub fn clear_conversation(
        &self,
        context: &TrustedRequestContext,
        agent_id: &AgentId,
        id: &ConversationId,
        expected_revision: u64,
    ) -> Result<Conversation, ApplicationError> {
        context.require(TASKS_SUBMIT)?;
        self.lifecycle_change(|state| {
            let tenant = tenant_mut(state, context);
            let item = conversation(tenant, context, agent_id, id)?;
            check_conversation_mutation(tenant, item, expected_revision)?;
            let replacement = new_conversation(
                require_active_agent(tenant, context, agent_id)?,
                item.title.clone(),
                context.received_at_ms,
            )?;
            tenant
                .conversations
                .get_mut(id)
                .ok_or(ApplicationError::ConversationNotFound)?
                .deleted_at_ms = Some(context.received_at_ms);
            tenant
                .conversations
                .insert(replacement.id.clone(), replacement.clone());
            Ok(replacement)
        })
    }
}

pub(super) fn prepare_turn(
    tenant: &TenantState,
    context: &TrustedRequestContext,
    request: &SubmitTask,
) -> Result<Option<(ConversationId, Vec<ConversationMessage>)>, ApplicationError> {
    if request
        .input
        .get("kind")
        .and_then(serde_json::Value::as_str)
        != Some("agent_conversation")
    {
        return Ok(None);
    }
    // Only the server may assemble history. Unknown input fields and client-written history
    // are refused, including empty `messages`, so they cannot alter idempotency semantics.
    let parsed = serde_json::from_value::<ConversationInput>(request.input.clone());
    let Ok(ConversationInput::AgentConversation {
        conversation_id,
        prompt,
        ..
    }) = parsed
    else {
        return Err(ApplicationError::Invalid(
            ValidationError::InvalidWebhookSchema,
        ));
    };
    if prompt.trim().is_empty() || request.input.get("messages").is_some() {
        return Err(ApplicationError::Invalid(
            ValidationError::InvalidWebhookSchema,
        ));
    }
    let item = conversation(tenant, context, &request.agent_id, &conversation_id)?;
    check_conversation_mutation(tenant, item, item.revision)?;
    let mut messages = Vec::new();
    let mut bytes = prompt.len();
    // Keep the most recent complete turns within a bounded context; never split a turn.
    for task in item
        .task_ids
        .iter()
        .rev()
        .filter_map(|id| tenant.tasks.get(id))
    {
        if task.status != TaskStatus::Succeeded || !task_owned_by(task, context) {
            continue;
        }
        let prompt = task
            .input
            .get("prompt")
            .and_then(serde_json::Value::as_str)
            .or_else(|| task.input.as_str())
            .unwrap_or_default();
        let Some(output) = &task.output else {
            continue;
        };
        bytes = bytes
            .saturating_add(prompt.len())
            .saturating_add(output.len());
        if bytes > 128 * 1024 || messages.len() >= 80 {
            break;
        }
        messages.push(ConversationMessage {
            role: ConversationRole::Assistant,
            content: output.clone(),
        });
        messages.push(ConversationMessage {
            role: ConversationRole::User,
            content: prompt.to_owned(),
        });
    }
    messages.reverse();
    Ok(Some((conversation_id, messages)))
}

pub(super) fn migrate_conversations(state: &mut State) -> Result<(), ApplicationError> {
    for tenant in state.tenants.values_mut() {
        let grouped: BTreeSet<_> = tenant
            .conversations
            .values()
            .flat_map(|item| item.task_ids.iter().cloned())
            .collect();
        for agent in tenant.agents.values() {
            let task_ids: Vec<_> = tenant
                .tasks
                .values()
                .filter(|task| {
                    task.agent_id == agent.id
                        && task.actor == agent.created_by
                        && task.input.get("kind").is_none()
                        && !grouped.contains(&task.id)
                })
                .map(|task| task.id.clone())
                .collect();
            if task_ids.is_empty() {
                continue;
            }
            let mut item = new_conversation(
                agent,
                "Earlier conversation".to_owned(),
                agent.created_at_ms,
            )?;
            item.task_ids = task_ids;
            tenant.conversations.insert(item.id.clone(), item);
        }
    }
    Ok(())
}
