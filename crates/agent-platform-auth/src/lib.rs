#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use agent_platform_core::{AttemptId, DelegationId, SubjectId, TenantId, ValidationError};
pub use connectors_client::operation;
use connectors_client::{HostedClient, RedeemedSubscription, SubscriptionLease};
use identity_client::{AccessCredential, IdentityClient};
use sha2::{Digest, Sha256};

pub const AGENTS_READ: &str = "agents.read";
pub const AGENTS_MANAGE: &str = "agents.manage";
pub const CAPABILITIES_READ: &str = "capabilities.read";
pub const CAPABILITIES_MANAGE: &str = "capabilities.manage";
pub const TASKS_READ: &str = "tasks.read";
pub const TASKS_SUBMIT: &str = "tasks.submit";
pub const TRIGGERS_READ: &str = "triggers.read";
pub const TRIGGERS_MANAGE: &str = "triggers.manage";
pub const CONNECTORS_AUDIENCE: &str = "urn:b10x:connectors";
pub const CONNECTORS_LEASE_SCOPE: &str = "connectors.credentials.lease";
pub const CONNECTORS_INVOKE_SCOPE: &str = "connectors.invoke";
const MODEL_LEASE_TTL: Duration = Duration::from_mins(30);
const MODEL_LEASE_USES: u16 = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedAuthority {
    tenant_id: TenantId,
    authority: SubjectId,
    executor: Option<SubjectId>,
    delegation_id: Option<DelegationId>,
    scopes: BTreeSet<String>,
}

impl VerifiedAuthority {
    pub fn new(
        tenant_id: TenantId,
        authority: SubjectId,
        executor: Option<SubjectId>,
        delegation_id: Option<DelegationId>,
        scopes: impl IntoIterator<Item = String>,
    ) -> Result<Self, AuthenticationError> {
        let scopes = scopes.into_iter().collect::<BTreeSet<_>>();
        if scopes.is_empty()
            || scopes.iter().any(|scope| {
                scope.is_empty()
                    || scope.len() > 128
                    || !scope
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte == b'.' || byte == b'_')
            })
        {
            return Err(AuthenticationError::new(
                "verified authority contains an invalid scope set",
            ));
        }
        let executor = executor.filter(|candidate| candidate != &authority);
        Ok(Self {
            tenant_id,
            authority,
            executor,
            delegation_id,
            scopes,
        })
    }

    pub fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub fn authority(&self) -> &SubjectId {
        &self.authority
    }

    pub fn executor(&self) -> Option<&SubjectId> {
        self.executor.as_ref()
    }

    pub fn delegation_id(&self) -> Option<&DelegationId> {
        self.delegation_id.as_ref()
    }

    pub fn permits(&self, scope: &str) -> bool {
        self.scopes.contains(scope)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{reason}")]
pub struct AuthenticationError {
    reason: String,
}

/// A finite capability over one user-bound model credential. It contains no Identity session or
/// Connectors access credential and can be redeemed only for its exact attempt.
pub struct UserModelLease {
    connectors: HostedClient,
    lease: SubscriptionLease,
    attempt_id: AttemptId,
}

impl std::fmt::Debug for UserModelLease {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UserModelLease")
            .field("lease_id", &self.lease.lease_id)
            .field("attempt_id", &self.attempt_id)
            .field("expires_at", &self.lease.expires_at)
            .finish_non_exhaustive()
    }
}

impl UserModelLease {
    pub fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }

    pub async fn redeem(&self) -> Result<RedeemedSubscription, AuthenticationError> {
        self.connectors
            .redeem_claude_code_subscription(&self.lease, self.attempt_id.as_str())
            .await
            .map_err(|_| AuthenticationError::new("the attempt model credential is unavailable"))
    }
}

/// Attempt-bounded authority to invoke Connector operations without exposing the Identity token.
#[derive(Clone)]
pub struct AttemptConnectorAccess {
    connectors: HostedClient,
    credential: AccessCredential,
    attempt_id: AttemptId,
}

impl std::fmt::Debug for AttemptConnectorAccess {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AttemptConnectorAccess")
            .field("attempt_id", &self.attempt_id)
            .finish_non_exhaustive()
    }
}

impl AttemptConnectorAccess {
    pub fn attempt_id(&self) -> &AttemptId {
        &self.attempt_id
    }

    /// Invoke through the exact attempt that received this short-lived authority.
    pub async fn invoke(
        &self,
        attempt_id: &AttemptId,
        context: &operation::OwnerContext,
        request: operation::InvokeRequest,
    ) -> Result<operation::OperationResult, AuthenticationError> {
        if attempt_id != &self.attempt_id {
            return Err(AuthenticationError::new(
                "Connector invocation authority belongs to another attempt",
            ));
        }
        let response = self
            .connectors
            .operation(
                self.credential.expose_at_authorization_boundary(),
                context,
                operation::OperationRequest::Invoke(request),
            )
            .await
            .map_err(|_| AuthenticationError::new("Connector invocation is unavailable"))?;
        match (response.status, response.response, response.error) {
            (operation::ResponseStatus::Ok, Some(result), None) => Ok(result),
            (operation::ResponseStatus::Error, None, Some(error)) => Err(AuthenticationError::new(
                format!("Connector invocation refused: {:?}", error.code),
            )),
            _ => Err(AuthenticationError::new(
                "Connector returned an invalid invocation response",
            )),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthenticatedRequest {
    authority: VerifiedAuthority,
    user_model_lease: Option<Arc<UserModelLease>>,
    connector_access: Option<Arc<AttemptConnectorAccess>>,
}

impl AuthenticatedRequest {
    pub fn authority(&self) -> &VerifiedAuthority {
        &self.authority
    }

    pub fn into_parts(
        self,
    ) -> (
        VerifiedAuthority,
        Option<Arc<UserModelLease>>,
        Option<Arc<AttemptConnectorAccess>>,
    ) {
        (self.authority, self.user_model_lease, self.connector_access)
    }
}

impl AuthenticationError {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl From<ValidationError> for AuthenticationError {
    fn from(error: ValidationError) -> Self {
        Self::new(error.to_string())
    }
}

pub trait CredentialVerifier: Send + Sync {
    fn verify<'a>(
        &'a self,
        authorization: Option<&'a str>,
        attempt_id: Option<&'a AttemptId>,
    ) -> Pin<Box<dyn Future<Output = Result<AuthenticatedRequest, AuthenticationError>> + Send + 'a>>;
}

/// Production verifier backed by Identity's exact-audience session authority endpoint.
#[derive(Clone)]
pub struct IdentityVerifier {
    client: IdentityClient,
    connectors: Option<HostedClient>,
    scopes: Vec<String>,
}

impl std::fmt::Debug for IdentityVerifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IdentityVerifier")
            .field("client", &self.client)
            .field(
                "connectors",
                &self.connectors.as_ref().map(|_| "configured"),
            )
            .field("scopes", &self.scopes)
            .finish()
    }
}

impl IdentityVerifier {
    pub fn new(
        identity_origin: &str,
        audience: &str,
        scopes: impl IntoIterator<Item = String>,
    ) -> Result<Self, AuthenticationError> {
        let scopes = scopes.into_iter().collect::<Vec<_>>();
        if scopes.is_empty() {
            return Err(AuthenticationError::new(
                "Identity verifier requires an authorization policy",
            ));
        }
        let client = IdentityClient::new(identity_origin, audience)
            .map_err(|error| AuthenticationError::new(error.to_string()))?;
        Ok(Self {
            client,
            connectors: None,
            scopes,
        })
    }

    pub fn with_connectors(mut self, hosted_api_base: &str) -> Result<Self, AuthenticationError> {
        self.connectors =
            Some(HostedClient::new(hosted_api_base).map_err(|_| {
                AuthenticationError::new("Connectors client configuration is invalid")
            })?);
        Ok(self)
    }
}

impl CredentialVerifier for IdentityVerifier {
    fn verify<'a>(
        &'a self,
        authorization: Option<&'a str>,
        attempt_id: Option<&'a AttemptId>,
    ) -> Pin<Box<dyn Future<Output = Result<AuthenticatedRequest, AuthenticationError>> + Send + 'a>>
    {
        Box::pin(async move {
            let authorization = authorization
                .ok_or_else(|| AuthenticationError::new("an Identity session is required"))?;
            let authority = self
                .client
                .resolve_session(authorization)
                .await
                .map_err(|error| AuthenticationError::new(error.to_string()))?;
            let authority = VerifiedAuthority::new(
                TenantId::new(authority.tenant_id)?,
                SubjectId::new(authority.subject)?,
                None,
                None,
                self.scopes.clone(),
            )?;
            let (user_model_lease, connector_access) = match (attempt_id, &self.connectors) {
                (Some(attempt_id), Some(connectors)) => {
                    let access = self
                        .client
                        .issue_access_token(
                            authorization,
                            CONNECTORS_AUDIENCE,
                            CONNECTORS_LEASE_SCOPE,
                        )
                        .await
                        .map_err(|_| {
                            AuthenticationError::new(
                                "Identity refused the Connectors lease authority",
                            )
                        })?;
                    let lease = connectors
                        .lease_claude_code_subscription(
                            access.credential.expose_at_authorization_boundary(),
                            attempt_id.as_str(),
                            MODEL_LEASE_TTL,
                            MODEL_LEASE_USES,
                        )
                        .await
                        .map_err(|_| {
                            AuthenticationError::new(
                                "a connected user model subscription is required",
                            )
                        })?;
                    let invoke_access = self
                        .client
                        .issue_access_token(
                            authorization,
                            CONNECTORS_AUDIENCE,
                            CONNECTORS_INVOKE_SCOPE,
                        )
                        .await
                        .map_err(|_| {
                            AuthenticationError::new(
                                "Identity refused the Connector invocation authority",
                            )
                        })?;
                    (
                        Some(Arc::new(UserModelLease {
                            connectors: connectors.clone(),
                            lease,
                            attempt_id: attempt_id.clone(),
                        })),
                        Some(Arc::new(AttemptConnectorAccess {
                            connectors: connectors.clone(),
                            credential: invoke_access.credential,
                            attempt_id: attempt_id.clone(),
                        })),
                    )
                }
                (Some(_), None) => {
                    return Err(AuthenticationError::new(
                        "user model execution is not configured",
                    ));
                }
                (None, _) => (None, None),
            };
            Ok(AuthenticatedRequest {
                authority,
                user_model_lease,
                connector_access,
            })
        })
    }
}

#[derive(Clone)]
pub struct DevelopmentVerifier {
    token_sha256: [u8; 32],
    authority: VerifiedAuthority,
}

impl std::fmt::Debug for DevelopmentVerifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DevelopmentVerifier")
            .field("token_sha256", &"[REDACTED]")
            .field("authority", &self.authority)
            .finish()
    }
}

impl DevelopmentVerifier {
    pub fn new(token: &str, authority: VerifiedAuthority) -> Result<Self, AuthenticationError> {
        if token.len() < 16 || token.len() > 4_096 || token.chars().any(char::is_whitespace) {
            return Err(AuthenticationError::new(
                "the development bearer token must be 16..=4096 non-whitespace characters",
            ));
        }
        Ok(Self {
            token_sha256: Sha256::digest(token.as_bytes()).into(),
            authority,
        })
    }

    fn matches(&self, authorization: Option<&str>) -> bool {
        let Some(token) = authorization.and_then(|value| value.strip_prefix("Bearer ")) else {
            return false;
        };
        let candidate: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        constant_time_equal(&self.token_sha256, &candidate)
    }
}

impl CredentialVerifier for DevelopmentVerifier {
    fn verify<'a>(
        &'a self,
        authorization: Option<&'a str>,
        _attempt_id: Option<&'a AttemptId>,
    ) -> Pin<Box<dyn Future<Output = Result<AuthenticatedRequest, AuthenticationError>> + Send + 'a>>
    {
        Box::pin(async move {
            if self.matches(authorization) {
                Ok(AuthenticatedRequest {
                    authority: self.authority.clone(),
                    user_model_lease: None,
                    connector_access: None,
                })
            } else {
                Err(AuthenticationError::new(
                    "a valid development bearer token is required",
                ))
            }
        })
    }
}

fn constant_time_equal(left: &[u8; 32], right: &[u8; 32]) -> bool {
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::{HeaderMap, HeaderValue, header};
    use axum::response::{IntoResponse, Response};
    use axum::routing::get;
    use serde_json::json;

    use super::*;

    fn authority() -> VerifiedAuthority {
        VerifiedAuthority::new(
            TenantId::new("tenant-one").unwrap(),
            SubjectId::new("human-alice").unwrap(),
            None,
            None,
            [AGENTS_MANAGE.to_owned()],
        )
        .unwrap()
    }

    #[tokio::test]
    async fn raw_credential_is_consumed_only_by_the_verifier() {
        let verifier = DevelopmentVerifier::new("a-development-secret", authority()).unwrap();
        let principal = verifier
            .verify(Some("Bearer a-development-secret"), None)
            .await
            .unwrap();
        assert_eq!(principal.authority().tenant_id().as_str(), "tenant-one");
        assert!(principal.authority().permits(AGENTS_MANAGE));
        assert!(
            verifier
                .verify(Some("Bearer another-development-secret"), None)
                .await
                .is_err()
        );
        assert!(!format!("{verifier:?}").contains("a-development-secret"));
    }

    #[test]
    fn equal_executor_is_not_a_delegation_shape() {
        let subject = SubjectId::new("human-alice").unwrap();
        let authority = VerifiedAuthority::new(
            TenantId::new("tenant-one").unwrap(),
            subject.clone(),
            Some(subject),
            None,
            [AGENTS_READ.to_owned()],
        )
        .unwrap();
        assert_eq!(authority.executor(), None);
    }

    async fn identity_authority(headers: HeaderMap) -> Response {
        assert_eq!(
            headers.get("x-b10x-audience"),
            Some(&HeaderValue::from_static("urn:b10x:agent-platform"))
        );
        let mut response = axum::Json(json!({
            "iss":"https://identity.example.test",
            "sub":"human-alice",
            "aud":"urn:b10x:agent-platform",
            "exp":4_102_444_800_i64,
            "email":"alice@example.test",
            "tenant_id":"tenant-one",
            "groups":["member"]
        }))
        .into_response();
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
            .headers_mut()
            .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
        response
    }

    #[tokio::test]
    async fn identity_authority_is_mapped_without_retaining_the_bearer() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().route("/v1/session-authority", get(identity_authority)),
            )
            .await
            .unwrap();
        });
        let verifier = IdentityVerifier::new(
            &format!("http://127.0.0.1:{}/", address.port()),
            "urn:b10x:agent-platform",
            [AGENTS_READ.to_owned(), TASKS_SUBMIT.to_owned()],
        )
        .unwrap();
        let authority = verifier
            .verify(Some("Bearer synthetic-session"), None)
            .await
            .unwrap();
        assert_eq!(authority.authority().tenant_id().as_str(), "tenant-one");
        assert_eq!(authority.authority().authority().as_str(), "human-alice");
        assert!(authority.authority().permits(TASKS_SUBMIT));
        assert!(!format!("{verifier:?}").contains("synthetic-session"));
    }
}
