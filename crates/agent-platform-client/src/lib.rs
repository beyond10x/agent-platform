#![forbid(unsafe_code)]

use std::time::Duration;

pub use agent_platform_core::{
    ActivateRevision, Agent, AgentId, AgentRevision, CapabilityProfileId, Conversation,
    ConversationId, ConversationRevision, CreateAgent, CreateCapabilityProfile, CreateConversation,
    PendingApproval, ResolveApproval, RevisionSpec, SubmitTask, Task, TaskId, UpdateAgent,
    UpdateCapabilityProfile, UpdateConversation,
};
use reqwest::header::{AUTHORIZATION, HeaderValue};
use serde::de::DeserializeOwned;
use url::Url;

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("invalid agent-platform client configuration")]
    Configuration,
    #[error("agent-platform request could not be completed")]
    Transport(#[source] reqwest::Error),
    #[error("agent-platform refused the request with status {0}")]
    Refused(u16),
    #[error("agent-platform refused the request with status {status}: {code}")]
    RefusedWithDetail {
        status: u16,
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct AgentPlatformClient {
    origin: Url,
    http: reqwest::Client,
}

impl AgentPlatformClient {
    pub async fn update_agent(
        &self,
        bearer: &str,
        id: &AgentId,
        request: &UpdateAgent,
    ) -> Result<Agent, ClientError> {
        self.patch_json(
            bearer,
            &format!("v1/agents/{id}", id = segment(id)?),
            request,
        )
        .await
    }

    pub async fn retire_agent(&self, bearer: &str, id: &AgentId) -> Result<(), ClientError> {
        self.delete(
            bearer,
            &format!("v1/agents/{id}", id = segment(id)?),
            None::<&ConversationRevision>,
        )
        .await
    }

    pub async fn retire_capability_profile(
        &self,
        bearer: &str,
        id: &CapabilityProfileId,
    ) -> Result<(), ClientError> {
        self.delete(
            bearer,
            &format!("v1/capability-profiles/{id}", id = segment(id)?),
            None::<&ConversationRevision>,
        )
        .await
    }

    pub async fn list_conversations(
        &self,
        bearer: &str,
        agent: &AgentId,
    ) -> Result<Vec<Conversation>, ClientError> {
        self.get_json(
            bearer,
            &format!("v1/agents/{agent}/conversations", agent = segment(agent)?),
        )
        .await
    }

    pub async fn create_conversation(
        &self,
        bearer: &str,
        agent: &AgentId,
        request: &CreateConversation,
    ) -> Result<Conversation, ClientError> {
        self.post_json(
            bearer,
            &format!("v1/agents/{agent}/conversations", agent = segment(agent)?),
            request,
        )
        .await
    }

    pub async fn update_conversation(
        &self,
        bearer: &str,
        agent: &AgentId,
        id: &ConversationId,
        request: &UpdateConversation,
    ) -> Result<Conversation, ClientError> {
        self.patch_json(
            bearer,
            &format!(
                "v1/agents/{agent}/conversations/{id}",
                agent = segment(agent)?,
                id = segment(id)?
            ),
            request,
        )
        .await
    }

    pub async fn delete_conversation(
        &self,
        bearer: &str,
        agent: &AgentId,
        id: &ConversationId,
        request: &ConversationRevision,
    ) -> Result<(), ClientError> {
        self.delete(
            bearer,
            &format!(
                "v1/agents/{agent}/conversations/{id}",
                agent = segment(agent)?,
                id = segment(id)?
            ),
            Some(request),
        )
        .await
    }

    pub async fn clear_conversation(
        &self,
        bearer: &str,
        agent: &AgentId,
        id: &ConversationId,
        request: &ConversationRevision,
    ) -> Result<Conversation, ClientError> {
        self.post_json(
            bearer,
            &format!(
                "v1/agents/{agent}/conversations/{id}/clear",
                agent = segment(agent)?,
                id = segment(id)?
            ),
            request,
        )
        .await
    }

    pub async fn list_conversation_tasks(
        &self,
        bearer: &str,
        agent: &AgentId,
        id: &ConversationId,
    ) -> Result<Vec<Task>, ClientError> {
        self.get_json(
            bearer,
            &format!(
                "v1/agents/{agent}/conversations/{id}/tasks",
                agent = segment(agent)?,
                id = segment(id)?
            ),
        )
        .await
    }

    async fn delete(
        &self,
        bearer: &str,
        path: &str,
        body: Option<&impl serde::Serialize>,
    ) -> Result<(), ClientError> {
        let mut request = self
            .http
            .delete(self.endpoint(path)?)
            .header(AUTHORIZATION, authorization(bearer)?);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.map_err(ClientError::Transport)?;
        checked_response(response).await?;
        Ok(())
    }
    pub fn new(origin: &str) -> Result<Self, ClientError> {
        let origin = Url::parse(origin).map_err(|_| ClientError::Configuration)?;
        let internal_http = origin.scheme() == "http"
            && origin.host_str().is_some_and(|host| {
                host == "127.0.0.1" || host == "localhost" || host.ends_with(".svc.cluster.local")
            });
        if !(origin.scheme() == "https" || internal_http)
            || origin.host_str().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
        {
            return Err(ClientError::Configuration);
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(35))
            .build()
            .map_err(ClientError::Transport)?;
        Ok(Self { origin, http })
    }

    pub async fn list_agents(&self, bearer: &str) -> Result<Vec<Agent>, ClientError> {
        self.get_json(bearer, "v1/agents").await
    }

    pub async fn get_agent(&self, bearer: &str, agent_id: &AgentId) -> Result<Agent, ClientError> {
        self.get_json(
            bearer,
            &format!("v1/agents/{agent_id}", agent_id = segment(agent_id)?),
        )
        .await
    }

    pub async fn create_agent(
        &self,
        bearer: &str,
        request: &CreateAgent,
    ) -> Result<Agent, ClientError> {
        self.post_json(bearer, "v1/agents", request).await
    }

    pub async fn create_revision(
        &self,
        bearer: &str,
        agent_id: &AgentId,
        request: &RevisionSpec,
    ) -> Result<AgentRevision, ClientError> {
        self.post_json(
            bearer,
            &format!(
                "v1/agents/{agent_id}/revisions",
                agent_id = segment(agent_id)?
            ),
            request,
        )
        .await
    }

    pub async fn list_revisions(
        &self,
        bearer: &str,
        agent_id: &AgentId,
    ) -> Result<Vec<AgentRevision>, ClientError> {
        self.get_json(
            bearer,
            &format!(
                "v1/agents/{agent_id}/revisions",
                agent_id = segment(agent_id)?
            ),
        )
        .await
    }

    pub async fn activate_revision(
        &self,
        bearer: &str,
        agent_id: &AgentId,
        request: &ActivateRevision,
    ) -> Result<Agent, ClientError> {
        self.post_json(
            bearer,
            &format!(
                "v1/agents/{agent_id}/activate",
                agent_id = segment(agent_id)?
            ),
            request,
        )
        .await
    }

    pub async fn list_capability_profiles(
        &self,
        bearer: &str,
    ) -> Result<serde_json::Value, ClientError> {
        self.get_json(bearer, "v1/capability-profiles").await
    }

    pub async fn create_capability_profile(
        &self,
        bearer: &str,
        request: &CreateCapabilityProfile,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(bearer, "v1/capability-profiles", request)
            .await
    }

    pub async fn update_capability_profile(
        &self,
        bearer: &str,
        profile_id: &CapabilityProfileId,
        request: &UpdateCapabilityProfile,
    ) -> Result<serde_json::Value, ClientError> {
        self.patch_json(
            bearer,
            &format!(
                "v1/capability-profiles/{profile_id}",
                profile_id = segment(profile_id)?
            ),
            request,
        )
        .await
    }

    pub async fn submit_task(
        &self,
        bearer: &str,
        request: &SubmitTask,
    ) -> Result<Task, ClientError> {
        self.post_json(bearer, "v1/tasks", request).await
    }

    /// Submit one task whose input is a typed coding-session-turn envelope.
    pub async fn submit_coding_session_turn(
        &self,
        bearer: &str,
        request: &SubmitTask,
    ) -> Result<Task, ClientError> {
        self.post_json(bearer, "v1/coding-session-turns", request)
            .await
    }

    pub async fn list_tasks(&self, bearer: &str) -> Result<Vec<Task>, ClientError> {
        self.get_json(bearer, "v1/tasks").await
    }

    pub async fn get_task(&self, bearer: &str, task_id: &TaskId) -> Result<Task, ClientError> {
        self.get_json(
            bearer,
            &format!("v1/tasks/{task_id}", task_id = segment(task_id)?),
        )
        .await
    }

    pub async fn list_task_approvals(
        &self,
        bearer: &str,
        task_id: &TaskId,
    ) -> Result<Vec<PendingApproval>, ClientError> {
        self.get_json(
            bearer,
            &format!("v1/tasks/{task_id}/approvals", task_id = segment(task_id)?),
        )
        .await
    }

    pub async fn resolve_task_approval(
        &self,
        bearer: &str,
        task_id: &TaskId,
        approval_id: &agent_platform_core::ApprovalId,
        resolution: &ResolveApproval,
    ) -> Result<PendingApproval, ClientError> {
        self.post_json(
            bearer,
            &format!(
                "v1/tasks/{task_id}/approvals/{approval_id}",
                task_id = segment(task_id)?,
                approval_id = segment(approval_id)?
            ),
            resolution,
        )
        .await
    }

    /// Returns the bounded streaming response without buffering it. The caller owns SSE framing.
    pub async fn task_events(
        &self,
        bearer: &str,
        task_id: &TaskId,
    ) -> Result<reqwest::Response, ClientError> {
        let response = self
            .http
            .get(self.endpoint(&format!(
                "v1/tasks/{task_id}/events",
                task_id = segment(task_id)?
            ))?)
            .header(AUTHORIZATION, authorization(bearer)?)
            .send()
            .await
            .map_err(ClientError::Transport)?;
        require_success(response)
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        bearer: &str,
        path: &str,
    ) -> Result<T, ClientError> {
        let response = self
            .http
            .get(self.endpoint(path)?)
            .header(AUTHORIZATION, authorization(bearer)?)
            .send()
            .await
            .map_err(ClientError::Transport)?;
        decode(response).await
    }

    async fn post_json<T: DeserializeOwned>(
        &self,
        bearer: &str,
        path: &str,
        body: &impl serde::Serialize,
    ) -> Result<T, ClientError> {
        let response = self
            .http
            .post(self.endpoint(path)?)
            .header(AUTHORIZATION, authorization(bearer)?)
            .json(body)
            .send()
            .await
            .map_err(ClientError::Transport)?;
        decode(response).await
    }

    async fn patch_json<T: DeserializeOwned>(
        &self,
        bearer: &str,
        path: &str,
        body: &impl serde::Serialize,
    ) -> Result<T, ClientError> {
        let response = self
            .http
            .patch(self.endpoint(path)?)
            .header(AUTHORIZATION, authorization(bearer)?)
            .json(body)
            .send()
            .await
            .map_err(ClientError::Transport)?;
        decode(response).await
    }

    fn endpoint(&self, path: &str) -> Result<Url, ClientError> {
        self.origin
            .join(path)
            .map_err(|_| ClientError::Configuration)
    }
}

fn authorization(bearer: &str) -> Result<HeaderValue, ClientError> {
    HeaderValue::from_str(bearer).map_err(|_| ClientError::Configuration)
}

async fn decode<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, ClientError> {
    checked_response(response)
        .await?
        .json()
        .await
        .map_err(ClientError::Transport)
}

#[derive(serde::Deserialize)]
struct Problem {
    code: String,
    message: String,
}

async fn checked_response(
    mut response: reqwest::Response,
) -> Result<reqwest::Response, ClientError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        if bytes.len().saturating_add(chunk.len()) > 8192 {
            return Err(ClientError::Refused(status));
        }
        bytes.extend_from_slice(&chunk);
    }
    if let Ok(problem) = serde_json::from_slice::<Problem>(&bytes)
        && problem.code.len() <= 128
        && problem.message.len() <= 2048
    {
        return Err(ClientError::RefusedWithDetail {
            status,
            code: problem.code,
            message: problem.message,
        });
    }
    Err(ClientError::Refused(status))
}

fn require_success(response: reqwest::Response) -> Result<reqwest::Response, ClientError> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(ClientError::Refused(response.status().as_u16()))
    }
}

// Dynamic identifiers must remain exactly one URL path segment. In particular,
// dot segments must never be normalized into a different endpoint.
fn segment(value: &impl std::fmt::Display) -> Result<String, ClientError> {
    let value = value.to_string();
    if value.is_empty() || value == "." || value == ".." {
        return Err(ClientError::Configuration);
    }
    let mut url = Url::parse("https://example.test/").map_err(|_| ClientError::Configuration)?;
    url.path_segments_mut()
        .map_err(|()| ClientError::Configuration)?
        .push(&value);
    Ok(url.path()[1..].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_cannot_change_routes() {
        for invalid in ["", ".", ".."] {
            assert!(segment(&invalid).is_err());
        }
        let client = AgentPlatformClient::new("https://example.test/").unwrap();
        for raw in [
            "../agents/other",
            "x?delete=true",
            "x#fragment",
            "%2e%2e",
            "a/b",
        ] {
            let encoded = segment(&raw).unwrap();
            assert!(!encoded.contains('/'));
            let url = client
                .endpoint(&format!("v1/agents/{encoded}/conversations"))
                .unwrap();
            assert_eq!(url.path_segments().unwrap().count(), 4);
            assert!(url.query().is_none());
            assert!(url.fragment().is_none());
        }
    }
}
