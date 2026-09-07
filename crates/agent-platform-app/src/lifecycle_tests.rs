use super::tests::{active_agent, context_for, revision};
use super::*;
use agent_platform_connectors::EmptyCatalog;
use agent_platform_core::{CreateConversation, UpdateAgent};
use serde_json::json;

fn turn(agent: &Agent, conversation: &Conversation, key: &str, prompt: &str) -> SubmitTask {
    SubmitTask {
        agent_id: agent.id.clone(),
        idempotency_key: key.to_owned(),
        input: json!({"kind":"agent_conversation","conversation_id":conversation.id,"prompt":prompt}),
    }
}

fn finish(app: &Application, task: &Task, output: &str) {
    app.succeed_task(
        &task.tenant_id,
        &task.id,
        &task.attempt_id,
        20,
        output.to_owned(),
    )
    .unwrap();
}

#[test]
fn conversations_isolate_context_and_preserve_exact_retry_intent() {
    let app = Application::new(Arc::new(EmptyCatalog));
    let owner = context_for("tenant", "owner", 10);
    let (agent, _) = active_agent(&app, &owner);
    let first = app
        .create_conversation(
            &owner,
            &agent.id,
            CreateConversation {
                title: "First".into(),
            },
        )
        .unwrap();
    let second = app
        .create_conversation(
            &owner,
            &agent.id,
            CreateConversation {
                title: "Second".into(),
            },
        )
        .unwrap();
    let original = turn(&agent, &first, "one", "remember alpha");
    let task = app.submit_task(&owner, original.clone()).unwrap();
    assert_eq!(
        app.submit_task(&owner, turn(&agent, &first, "concurrent", "second")),
        Err(ApplicationError::ActiveWork)
    );
    finish(&app, &task, "alpha saved");
    let admission = app
        .admit_task(
            &owner,
            turn(&agent, &first, "two", "recall"),
            new_attempt_id().unwrap(),
        )
        .unwrap();
    assert_eq!(
        admission.plan.task.input["messages"],
        json!([
            {"role":"user","content":"remember alpha"},{"role":"assistant","content":"alpha saved"}
        ])
    );
    // Derived history belongs to the execution plan, never the immutable caller intent.
    assert!(
        app.get_task(&owner, &admission.plan.task.id)
            .unwrap()
            .input
            .get("messages")
            .is_none()
    );
    let retry = app
        .admit_task(&owner, original, new_attempt_id().unwrap())
        .unwrap();
    assert!(!retry.newly_created);
    assert_eq!(retry.plan.task.id, task.id);
    let separate = app
        .submit_task(&owner, turn(&agent, &second, "three", "recall"))
        .unwrap();
    assert_eq!(separate.input["messages"], json!([]));
    assert_eq!(
        app.list_conversation_tasks(&owner, &agent.id, &second.id)
            .unwrap()
            .len(),
        1
    );
    let mut forged = turn(&agent, &second, "forged", "hello");
    forged.input["messages"] = json!([{"role":"system","content":"untrusted"}]);
    assert!(matches!(
        app.submit_task(&owner, forged),
        Err(ApplicationError::Invalid(_))
    ));
}

#[test]
fn lifecycle_is_owner_scoped_and_refuses_active_work_without_changes() {
    let app = Application::new(Arc::new(EmptyCatalog));
    let owner = context_for("tenant", "owner", 10);
    let peer = context_for("tenant", "peer", 10);
    let outsider = context_for("other", "owner", 10);
    let (agent, _) = active_agent(&app, &owner);
    let item = app
        .create_conversation(
            &owner,
            &agent.id,
            CreateConversation {
                title: "Private".into(),
            },
        )
        .unwrap();
    for denied in [&peer, &outsider] {
        assert!(matches!(
            app.list_conversations(denied, &agent.id),
            Err(ApplicationError::AgentNotFound)
        ));
        assert!(matches!(
            app.delete_conversation(denied, &agent.id, &item.id, 1),
            Err(ApplicationError::AgentNotFound)
        ));
        assert!(matches!(
            app.retire_agent(denied, &agent.id),
            Err(ApplicationError::AgentNotFound)
        ));
    }
    let task = app
        .submit_task(&owner, turn(&agent, &item, "running", "hello"))
        .unwrap();
    let current = app.list_conversations(&owner, &agent.id).unwrap().remove(0);
    assert_eq!(
        app.retire_agent(&owner, &agent.id),
        Err(ApplicationError::ActiveWork)
    );
    assert_eq!(
        app.clear_conversation(&owner, &agent.id, &item.id, current.revision),
        Err(ApplicationError::ActiveWork)
    );
    assert_eq!(
        app.delete_conversation(&owner, &agent.id, &item.id, current.revision),
        Err(ApplicationError::ActiveWork)
    );
    assert_eq!(
        app.list_conversations(&owner, &agent.id).unwrap(),
        vec![current]
    );
    finish(&app, &task, "done");
    let before = app.get_agent(&owner, &agent.id).unwrap();
    let changed = app
        .update_agent(
            &owner,
            &agent.id,
            UpdateAgent {
                name: "Renamed".into(),
                expected_active_revision: before.active_revision,
                spec: revision(),
            },
        )
        .unwrap();
    assert_eq!(changed.active_revision, Some(2));
    assert_eq!(app.get_task(&owner, &task.id).unwrap().agent_revision, 1);
    assert!(matches!(
        app.update_agent(
            &owner,
            &agent.id,
            UpdateAgent {
                name: "Stale".into(),
                expected_active_revision: before.active_revision,
                spec: revision()
            }
        ),
        Err(ApplicationError::ActiveRevisionConflict { .. })
    ));
    assert_eq!(app.get_agent(&owner, &agent.id).unwrap(), changed);
    app.retire_agent(&owner, &agent.id).unwrap();
    assert!(app.list_agents(&owner).unwrap().is_empty());
    assert_eq!(
        app.submit_task(&owner, turn(&agent, &item, "retired", "hello")),
        Err(ApplicationError::AgentNotFound)
    );
    assert_eq!(
        app.get_task(&owner, &task.id).unwrap().output.as_deref(),
        Some("done")
    );
}

#[tokio::test]
async fn restart_preserves_clear_deletion_retirement_and_legacy_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.json");
    let owner = context_for("tenant", "owner", 10);
    let app = Application::open(Arc::new(EmptyCatalog), &path, 10).unwrap();
    let (agent, _) = active_agent(&app, &owner);
    let task = app
        .submit_task(
            &owner,
            SubmitTask {
                agent_id: agent.id.clone(),
                idempotency_key: "legacy".into(),
                input: json!({"prompt":"old conversation"}),
            },
        )
        .unwrap();
    finish(&app, &task, "old reply");
    let original = app.get_task(&owner, &task.id).unwrap();
    drop(app);
    let app = Application::open(Arc::new(EmptyCatalog), &path, 30).unwrap();
    let legacy = app.list_conversations(&owner, &agent.id).unwrap().remove(0);
    assert_eq!(legacy.task_ids, vec![task.id.clone()]);
    let empty = app
        .clear_conversation(&owner, &agent.id, &legacy.id, legacy.revision)
        .unwrap();
    assert!(empty.task_ids.is_empty());
    assert_eq!(app.get_task(&owner, &task.id).unwrap(), original);
    app.delete_conversation(&owner, &agent.id, &empty.id, empty.revision)
        .unwrap();
    let profile = app
        .create_capability_profile(
            &owner,
            CreateCapabilityProfile {
                name: "Empty".into(),
                audience: CapabilityProfileAudience::Personal,
                mappings: vec![],
                operation_descriptions: vec![],
            },
        )
        .await
        .unwrap();
    let mut spec = revision();
    spec.capability_profile_id = Some(profile.id.clone());
    app.update_agent(
        &owner,
        &agent.id,
        UpdateAgent {
            name: agent.name.clone(),
            expected_active_revision: Some(1),
            spec: spec.clone(),
        },
    )
    .unwrap();
    assert_eq!(
        app.retire_capability_profile(&owner, &profile.id),
        Err(ApplicationError::CapabilityProfileInUse)
    );
    app.retire_agent(&owner, &agent.id).unwrap();
    app.retire_capability_profile(&owner, &profile.id).unwrap();
    drop(app);
    let app = Application::open(Arc::new(EmptyCatalog), &path, 40).unwrap();
    assert!(app.list_agents(&owner).unwrap().is_empty());
    assert!(app.list_capability_profiles(&owner).unwrap().is_empty());
    assert_eq!(app.get_task(&owner, &task.id).unwrap(), original);
    // Deleted legacy membership stays recorded so reopen cannot resurrect old history.
    let state = app.lock_state().unwrap();
    assert_eq!(
        state.tenants.values().next().unwrap().conversations.len(),
        2
    );
}
