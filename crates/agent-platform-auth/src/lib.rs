#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::Pin;

use agent_platform_core::{DelegationId, SubjectId, TenantId, ValidationError};
use sha2::{Digest, Sha256};

pub const AGENTS_READ: &str = "agents.read";
pub const AGENTS_MANAGE: &str = "agents.manage";
pub const CAPABILITIES_READ: &str = "capabilities.read";
pub const CAPABILITIES_MANAGE: &str = "capabilities.manage";
pub const TASKS_READ: &str = "tasks.read";
pub const TASKS_SUBMIT: &str = "tasks.submit";
pub const TRIGGERS_READ: &str = "triggers.read";
pub const TRIGGERS_MANAGE: &str = "triggers.manage";

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
    ) -> Pin<Box<dyn Future<Output = Result<VerifiedAuthority, AuthenticationError>> + Send + 'a>>;
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
    ) -> Pin<Box<dyn Future<Output = Result<VerifiedAuthority, AuthenticationError>> + Send + 'a>>
    {
        Box::pin(async move {
            if self.matches(authorization) {
                Ok(self.authority.clone())
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
            .verify(Some("Bearer a-development-secret"))
            .await
            .unwrap();
        assert_eq!(principal.tenant_id().as_str(), "tenant-one");
        assert!(principal.permits(AGENTS_MANAGE));
        assert!(
            verifier
                .verify(Some("Bearer another-development-secret"))
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
}
