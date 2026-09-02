#![forbid(unsafe_code)]

use std::time::Duration;

pub use agent_platform_core::{
    ActivateRevision, Agent, AgentId, AgentRevision, CapabilityProfileId, CreateAgent,
    CreateCapabilityProfile, PendingApproval, ResolveApproval, RevisionSpec, SubmitTask, Task,
    TaskId, UpdateCapabilityProfile,
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
}

#[derive(Debug, Clone)]
pub struct AgentPlatformClient {
    origin: Url,
    http: reqwest::Client,
}

impl AgentPlatformClient {
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
        self.get_json(bearer, &format!("v1/agents/{agent_id}"))
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
        self.post_json(bearer, &format!("v1/agents/{agent_id}/revisions"), request)
            .await
    }

    pub async fn list_revisions(
        &self,
        bearer: &str,
        agent_id: &AgentId,
    ) -> Result<Vec<AgentRevision>, ClientError> {
        self.get_json(bearer, &format!("v1/agents/{agent_id}/revisions"))
            .await
    }

    pub async fn activate_revision(
        &self,
        bearer: &str,
        agent_id: &AgentId,
        request: &ActivateRevision,
    ) -> Result<Agent, ClientError> {
        self.post_json(bearer, &format!("v1/agents/{agent_id}/activate"), request)
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
            &format!("v1/capability-profiles/{profile_id}"),
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

    pub async fn get_task(&self, bearer: &str, task_id: &TaskId) -> Result<Task, ClientError> {
        self.get_json(bearer, &format!("v1/tasks/{task_id}")).await
    }

    pub async fn list_task_approvals(
        &self,
        bearer: &str,
        task_id: &TaskId,
    ) -> Result<Vec<PendingApproval>, ClientError> {
        self.get_json(bearer, &format!("v1/tasks/{task_id}/approvals"))
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
            &format!("v1/tasks/{task_id}/approvals/{approval_id}"),
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
            .get(self.endpoint(&format!("v1/tasks/{task_id}/events"))?)
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
    require_success(response)?
        .json()
        .await
        .map_err(ClientError::Transport)
}

fn require_success(response: reqwest::Response) -> Result<reqwest::Response, ClientError> {
    if response.status().is_success() {
        Ok(response)
    } else {
        Err(ClientError::Refused(response.status().as_u16()))
    }
}
