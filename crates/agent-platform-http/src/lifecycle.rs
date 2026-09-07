use super::{HttpState, application_error, result};
use agent_platform_app::{ApplicationError, TrustedRequestContext};
use agent_platform_core::{AgentId, CapabilityProfileId};
use agent_platform_core::{
    ConversationId, ConversationRevision, CreateConversation, UpdateAgent, UpdateConversation,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

pub(super) async fn update_agent(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path(id): Path<AgentId>,
    Json(request): Json<UpdateAgent>,
) -> Response {
    result(
        StatusCode::OK,
        state.app.update_agent(&context, &id, request),
    )
}

fn removed(result: Result<(), ApplicationError>) -> Response {
    match result {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => application_error(&error),
    }
}

pub(super) async fn retire_agent(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path(id): Path<AgentId>,
) -> Response {
    removed(state.app.retire_agent(&context, &id))
}

pub(super) async fn retire_capability_profile(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path(id): Path<CapabilityProfileId>,
) -> Response {
    removed(state.app.retire_capability_profile(&context, &id))
}

pub(super) async fn list_conversations(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path(id): Path<AgentId>,
) -> Response {
    result(StatusCode::OK, state.app.list_conversations(&context, &id))
}

pub(super) async fn create_conversation(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path(id): Path<AgentId>,
    Json(request): Json<CreateConversation>,
) -> Response {
    result(
        StatusCode::CREATED,
        state.app.create_conversation(&context, &id, request),
    )
}

pub(super) async fn update_conversation(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path((agent, id)): Path<(AgentId, ConversationId)>,
    Json(request): Json<UpdateConversation>,
) -> Response {
    result(
        StatusCode::OK,
        state
            .app
            .update_conversation(&context, &agent, &id, request),
    )
}

pub(super) async fn delete_conversation(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path((agent, id)): Path<(AgentId, ConversationId)>,
    Json(request): Json<ConversationRevision>,
) -> Response {
    removed(
        state
            .app
            .delete_conversation(&context, &agent, &id, request.expected_revision),
    )
}

pub(super) async fn clear_conversation(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path((agent, id)): Path<(AgentId, ConversationId)>,
    Json(request): Json<ConversationRevision>,
) -> Response {
    result(
        StatusCode::CREATED,
        state
            .app
            .clear_conversation(&context, &agent, &id, request.expected_revision),
    )
}

pub(super) async fn list_conversation_tasks(
    State(state): State<HttpState>,
    Extension(context): Extension<TrustedRequestContext>,
    Path((agent, id)): Path<(AgentId, ConversationId)>,
) -> Response {
    result(
        StatusCode::OK,
        state.app.list_conversation_tasks(&context, &agent, &id),
    )
}
