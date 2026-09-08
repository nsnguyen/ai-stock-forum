mod support;

use std::sync::Arc;

use ai_stock_forum::domain::{
    Actor, AgentProfileVersionId, CommandId, CorrelationId, Digest, EventId, IdGenerator,
    MemoryEntryId, MemoryEntryVersionId, MemoryNamespaceId, ObjectRef, canonical_json_bytes,
};
use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentProfileVersion, AgentRole},
    app::{
        AgentProfileSelector, ApplicationCommand, ApplicationService, AuthorizationDecision,
        CommandView, MemoryEditPreview, ShutdownReason,
    },
    config::AppPaths,
    memory::{
        EpisodicSourceRef, EpisodicSummary, ExpectedMemoryEntryState, MemoryEntryDraft,
        MemoryEntryVersion, MemoryProposalOperation, MemoryProposalRef, MemoryPurposeScope,
        MemoryRetrievalBudget, MemoryRetrievalRequest, MemoryRetrievalScope,
    },
    persistence::{Database, EventRepository, MemoryRepository, ProjectionRepository},
    policy::ApprovalStatus,
    recovery::{ProjectionState, reduce},
};
use uuid::Uuid;

fn persistent_profile(fixture: &support::PersistentFixture) -> AgentProfileVersion {
    let mut app = fixture.service();
    let created = app
        .execute_user(ApplicationCommand::CreateAgentProfile {
            draft: AgentProfileDraft::new(
                "Episodic recovery".into(),
                "Test-only source-linked summary owner.".into(),
                AgentRole::Custom,
                "research".into(),
                vec![],
                "Careful.".into(),
                "Verify sources.".into(),
                AgentBindings::default(),
                vec![],
                vec![],
            )
            .unwrap(),
            template_provenance: None,
        })
        .unwrap();
    let CommandView::AgentProfileCreated(created) = created.view else {
        panic!("expected profile creation");
    };
    app.finish(ShutdownReason::UserQuit).unwrap();
    drop(app);
    fixture.active_profile(created.profile_id)
}

fn propose(
    app: &mut ApplicationService,
    id: u128,
    profile: &AgentProfileVersion,
    key: &str,
    value: &str,
) -> MemoryProposalRef {
    let outcome = app
        .execute(envelope(
            id,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ExpectedMemoryEntryState::Absent,
                operation: MemoryProposalOperation::Set {
                    candidate: MemoryEntryDraft::new(key.into(), value.into(), vec![]).unwrap(),
                },
                rationale: "Authenticated recovery fixture.".into(),
            },
        ))
        .unwrap();
    match outcome.view {
        CommandView::MemoryProposalCreated(view) => view.proposal,
        other => panic!("expected proposal creation, got {other:?}"),
    }
}

fn resolve_proposal(
    app: &mut ApplicationService,
    id: u128,
    proposal: MemoryProposalRef,
    accept: bool,
) {
    let review = if accept {
        app.preview_memory_proposal_approval(proposal.clone())
            .unwrap()
    } else {
        app.preview_memory_proposal_rejection(proposal.clone())
            .unwrap()
    };
    let command = if accept {
        ApplicationCommand::ApproveMemoryProposal {
            proposal,
            approval_id: review.approval_id,
            expected_approval_status: ApprovalStatus::Pending,
            expected_entry: review.expected_entry,
            review_token: review.review_token,
            review_digest: review.review_digest,
        }
    } else {
        ApplicationCommand::RejectMemoryProposal {
            proposal,
            approval_id: review.approval_id,
            expected_approval_status: ApprovalStatus::Pending,
            expected_entry: review.expected_entry,
            review_token: review.review_token,
            review_digest: review.review_digest,
        }
    };
    app.execute(envelope(id, Actor::Human, command)).unwrap();
}

fn verified_memory_history() -> Vec<ai_stock_forum::app::EventEnvelope> {
    let mut fixture = support::persistent_fixture();
    let profile = persistent_profile(&fixture);
    let mut app = fixture.service();

    for (id, value) in [(200, "first"), (201, "second")] {
        let review = match app
            .preview_memory_set(
                AgentProfileSelector::from(profile.profile_id()),
                MemoryEntryDraft::new("versioned key".into(), value.into(), vec![]).unwrap(),
            )
            .unwrap()
        {
            MemoryEditPreview::Review(review) => review,
            other => panic!("expected memory review, got {other:?}"),
        };
        app.execute(envelope(
            id,
            Actor::Human,
            ApplicationCommand::SetMemoryEntry {
                profile: review.profile,
                expected: review.expected,
                candidate: review.candidate.unwrap(),
                review_token: review.review_token,
                review_digest: review.review_digest,
            },
        ))
        .unwrap();
    }

    let accepted = propose(&mut app, 210, &profile, "accepted key", "accepted");
    resolve_proposal(&mut app, 211, accepted, true);
    let _pending = propose(&mut app, 212, &profile, "pending key", "pending");
    let rejected = propose(&mut app, 213, &profile, "rejected key", "rejected");
    resolve_proposal(&mut app, 214, rejected, false);
    let selected = propose(&mut app, 215, &profile, "sibling key", "selected");
    let _expired = propose(&mut app, 216, &profile, "sibling key", "expired");
    resolve_proposal(&mut app, 217, selected, true);

    app.finish(ShutdownReason::UserQuit).unwrap();
    drop(app);
    let sources = fixture
        .events()
        .into_iter()
        .filter(|event| {
            matches!(
                event.kind.as_str(),
                "agent_profile_created" | "memory_entry_set" | "memory_proposal_created"
            )
        })
        .take(3)
        .map(|event| event.event_id)
        .collect::<Vec<_>>();
    assert_eq!(sources.len(), 3);
    support::record_test_episodic_summary(
        &mut fixture,
        profile.reference(),
        "Recovery matrix".into(),
        "Authenticated summary body.".into(),
        vec!["matrix".into()],
        sources,
    )
    .unwrap();
    let database = fixture.open_database();
    EventRepository::load_all(database.connection()).unwrap()
}

fn event_only_database(
    events: &[ai_stock_forum::app::EventEnvelope],
) -> (tempfile::TempDir, AppPaths, Database) {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let mut database = Database::open(&paths).unwrap();
    let tx = database.immediate_transaction().unwrap();
    for event in events {
        let appended = EventRepository::append(
            &tx,
            ai_stock_forum::app::PendingEvent {
                event_id: event.event_id,
                event_schema_version: event.event_schema_version,
                actor: event.actor.clone(),
                occurred_at_ms: event.occurred_at_ms,
                correlation_id: event.correlation_id,
                causation_id: event.causation_id,
                object: event.object.clone(),
                event: event.event.clone(),
            },
        )
        .unwrap();
        assert_eq!(&appended, event);
    }
    tx.commit().unwrap();
    (temporary_directory, paths, database)
}

fn recovered_database(
    events: &[ai_stock_forum::app::EventEnvelope],
) -> (tempfile::TempDir, AppPaths, Database) {
    let (temporary_directory, paths, mut database) = event_only_database(events);
    ProjectionRepository::rebuild(database.connection_mut(), events).unwrap();
    (temporary_directory, paths, database)
}

fn current_projection_rows(database: &Database) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
    let mut entries = database
        .connection()
        .prepare(
            "SELECT memory_namespace_id,normalized_key,entry_id,entry_version_id,version,state,content_digest
             FROM current_memory_entries ORDER BY memory_namespace_id,normalized_key",
        )
        .unwrap();
    let entries = entries
        .query_map([], |row| {
            Ok(vec![
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get::<_, i64>(4)?.to_string(),
                row.get(5)?,
                row.get(6)?,
            ])
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut statuses = database
        .connection()
        .prepare(
            "SELECT proposal_id,proposal_version,proposal_content_digest,memory_namespace_id,
                    normalized_key,status,COALESCE(resolution_event_id,''),created_at_ms
             FROM current_memory_proposal_status ORDER BY proposal_id",
        )
        .unwrap();
    let statuses = statuses
        .query_map([], |row| {
            Ok(vec![
                row.get(0)?,
                row.get::<_, i64>(1)?.to_string(),
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get::<_, i64>(7)?.to_string(),
            ])
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    (entries, statuses)
}

fn assert_recovery_rejects_each_tamper(
    events: &[ai_stock_forum::app::EventEnvelope],
    disable_guards: &str,
    mutations: &[&str],
) {
    let (_temporary_directory, _paths, mut database) = recovered_database(events);
    database
        .connection()
        .execute_batch(&format!(
            "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON; {disable_guards}"
        ))
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    for mutation in mutations {
        tx.transaction().execute_batch("SAVEPOINT tamper").unwrap();
        tx.transaction().execute(mutation, []).unwrap();
        let error = ProjectionRepository::rebuild_in(&tx, events).expect_err(mutation);
        assert_eq!(error.code(), "invalid_event_record", "{mutation}");
        tx.transaction()
            .execute_batch("ROLLBACK TO tamper; RELEASE tamper")
            .unwrap();
    }
    tx.rollback().unwrap();
}

#[test]
fn recovery_rejects_every_immutable_entry_column_tamper() {
    let events = verified_memory_history();
    assert_recovery_rejects_each_tamper(
        &events,
        "DROP TRIGGER memory_entry_versions_no_update;",
        &[
            "UPDATE memory_entry_versions SET memory_namespace_id='00000000-0000-0000-0000-000000990001' WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET entry_id='00000000-0000-0000-0000-000000990002' WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET entry_version_id='00000000-0000-0000-0000-000000990003' WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET version=99 WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET predecessor_version_id='00000000-0000-0000-0000-000000990004' WHERE version=2",
            "UPDATE memory_entry_versions SET content_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET display_key='Changed' WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET normalized_key='changed' WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET state=CASE state WHEN 'present' THEN 'deleted' ELSE 'present' END WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET value_text='changed private value' WHERE state='present' AND rowid=(SELECT MIN(rowid) FROM memory_entry_versions WHERE state='present')",
            "UPDATE memory_entry_versions SET value_bytes=value_bytes+1 WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET purpose_tags_json=CAST('[\"changed\"]' AS BLOB) WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET created_by_kind='system' WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET created_by_id='00000000-0000-0000-0000-000000990005' WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET accepted_proposal_id='00000000-0000-0000-0000-000000990006' WHERE accepted_proposal_id IS NOT NULL",
            "UPDATE memory_entry_versions SET accepted_proposal_version=accepted_proposal_version+1 WHERE accepted_proposal_version IS NOT NULL",
            "UPDATE memory_entry_versions SET accepted_proposal_digest='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' WHERE accepted_proposal_digest IS NOT NULL",
            "UPDATE memory_entry_versions SET plaintext_validation_version=plaintext_validation_version+1 WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET creation_event_sequence=creation_event_sequence+1 WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET creation_event_id='00000000-0000-0000-0000-000000990007' WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET record_json=CAST('{}' AS BLOB) WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET created_at_ms=created_at_ms+1 WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
            "UPDATE memory_entry_versions SET record_digest='cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc' WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
        ],
    );
}

#[test]
fn recovery_rejects_every_immutable_proposal_column_tamper() {
    let events = verified_memory_history();
    assert_recovery_rejects_each_tamper(
        &events,
        "DROP TRIGGER memory_proposals_no_update;",
        &[
            "UPDATE memory_proposals SET proposal_id='00000000-0000-0000-0000-000000991001' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET version=version+1 WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET proposer_profile_id='00000000-0000-0000-0000-000000991002' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET proposer_profile_version_id='00000000-0000-0000-0000-000000991003' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET proposer_profile_version=proposer_profile_version+1 WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET proposer_profile_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET memory_namespace_id='00000000-0000-0000-0000-000000991004' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET operation=CASE operation WHEN 'set' THEN 'delete' ELSE 'set' END WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET display_key='Changed' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET normalized_key='changed' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET expected_kind='present' WHERE expected_kind='absent' AND rowid=(SELECT MIN(rowid) FROM memory_proposals WHERE expected_kind='absent')",
            "UPDATE memory_proposals SET expected_entry_id='00000000-0000-0000-0000-000000991005' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET expected_entry_version_id='00000000-0000-0000-0000-000000991006' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET expected_entry_version=1 WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET expected_entry_digest='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET candidate_value='changed' WHERE candidate_value IS NOT NULL AND rowid=(SELECT MIN(rowid) FROM memory_proposals WHERE candidate_value IS NOT NULL)",
            "UPDATE memory_proposals SET candidate_value_bytes=candidate_value_bytes+1 WHERE candidate_value_bytes IS NOT NULL AND rowid=(SELECT MIN(rowid) FROM memory_proposals WHERE candidate_value_bytes IS NOT NULL)",
            "UPDATE memory_proposals SET candidate_purpose_tags_json=CAST('[\"changed\"]' AS BLOB) WHERE candidate_purpose_tags_json IS NOT NULL AND rowid=(SELECT MIN(rowid) FROM memory_proposals WHERE candidate_purpose_tags_json IS NOT NULL)",
            "UPDATE memory_proposals SET rationale='changed' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET plaintext_validation_version=plaintext_validation_version+1 WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET created_at_ms=created_at_ms+1 WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET creation_event_sequence=creation_event_sequence+1 WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET creation_event_id='00000000-0000-0000-0000-000000991007' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET approval_id='00000000-0000-0000-0000-000000991008' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET content_digest='cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET record_digest='dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
            "UPDATE memory_proposals SET record_json=CAST('{}' AS BLOB) WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
        ],
    );
}

#[test]
fn recovery_rejects_every_resolution_and_memory_approval_column_tamper() {
    let events = verified_memory_history();
    assert_recovery_rejects_each_tamper(
        &events,
        "DROP TRIGGER memory_proposal_resolutions_no_update;
         DROP TRIGGER approval_records_identity_guard;
         DROP TRIGGER approval_records_transition_guard;",
        &[
            "UPDATE memory_proposal_resolutions SET proposal_id='00000000-0000-0000-0000-000000992001' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposal_resolutions)",
            "UPDATE memory_proposal_resolutions SET proposal_version=proposal_version+1 WHERE rowid=(SELECT MIN(rowid) FROM memory_proposal_resolutions)",
            "UPDATE memory_proposal_resolutions SET proposal_content_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposal_resolutions)",
            "UPDATE memory_proposal_resolutions SET status=CASE status WHEN 'accepted' THEN 'rejected' ELSE 'accepted' END WHERE rowid=(SELECT MIN(rowid) FROM memory_proposal_resolutions)",
            "UPDATE memory_proposal_resolutions SET approval_id='00000000-0000-0000-0000-000000992002' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposal_resolutions)",
            "UPDATE memory_proposal_resolutions SET resolved_by_kind='system' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposal_resolutions)",
            "UPDATE memory_proposal_resolutions SET resolved_by_id='00000000-0000-0000-0000-000000992003' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposal_resolutions)",
            "UPDATE memory_proposal_resolutions SET resolved_at_ms=resolved_at_ms+1 WHERE rowid=(SELECT MIN(rowid) FROM memory_proposal_resolutions)",
            "UPDATE memory_proposal_resolutions SET resolution_event_sequence=resolution_event_sequence+1 WHERE rowid=(SELECT MIN(rowid) FROM memory_proposal_resolutions)",
            "UPDATE memory_proposal_resolutions SET resolution_event_id='00000000-0000-0000-0000-000000992004' WHERE rowid=(SELECT MIN(rowid) FROM memory_proposal_resolutions)",
            "UPDATE memory_proposal_resolutions SET resolution_json=CAST('{}' AS BLOB) WHERE rowid=(SELECT MIN(rowid) FROM memory_proposal_resolutions)",
            "UPDATE approval_records SET approval_id='00000000-0000-0000-0000-000000992005' WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET action_kind='installation_change' WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET object_kind='changed' WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET object_id='00000000-0000-0000-0000-000000992006' WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET object_version=object_version+1 WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET object_digest='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb' WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET actor_kind='human' WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET actor_kind='system' WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET actor_kind='agent',actor_id=NULL WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET actor_id='00000000-0000-0000-0000-000000992007' WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET status=CASE status WHEN 'pending' THEN 'accepted' ELSE 'pending' END WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET created_at_ms=created_at_ms+1 WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET expires_at_ms=created_at_ms+1 WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET resolved_at_ms=COALESCE(resolved_at_ms,created_at_ms)+1 WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET resolution_kind=CASE COALESCE(resolution_kind,'') WHEN 'accepted' THEN 'rejected' ELSE 'accepted' END WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET resolution_event_id='00000000-0000-0000-0000-000000992008' WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET resolution_actor_kind='system' WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET resolution_actor_id='00000000-0000-0000-0000-000000992009' WHERE action_kind='memory_mutation' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
            "UPDATE approval_records SET resolution_actor_kind='human',resolution_actor_id='00000000-0000-0000-0000-000000992010' WHERE action_kind='memory_mutation' AND status<>'pending' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation' AND status<>'pending')",
            "UPDATE approval_records SET resolution_actor_kind='agent',resolution_actor_id=NULL WHERE action_kind='memory_mutation' AND status<>'pending' AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation' AND status<>'pending')",
        ],
    );
}

#[test]
fn recovery_rejects_every_summary_and_source_column_tamper() {
    let events = verified_memory_history();
    assert_recovery_rejects_each_tamper(
        &events,
        "DROP TRIGGER episodic_summaries_no_update;
         DROP TRIGGER episodic_summary_sources_no_update;",
        &[
            "UPDATE episodic_summaries SET summary_id='00000000-0000-0000-0000-000000993001'",
            "UPDATE episodic_summaries SET version=version+1",
            "UPDATE episodic_summaries SET memory_namespace_id='00000000-0000-0000-0000-000000993002'",
            "UPDATE episodic_summaries SET profile_id='00000000-0000-0000-0000-000000993003'",
            "UPDATE episodic_summaries SET profile_version_id='00000000-0000-0000-0000-000000993004'",
            "UPDATE episodic_summaries SET profile_version=profile_version+1",
            "UPDATE episodic_summaries SET profile_content_digest='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'",
            "UPDATE episodic_summaries SET label='changed'",
            "UPDATE episodic_summaries SET body='changed'",
            "UPDATE episodic_summaries SET purpose_tags_json=CAST('[\"changed\"]' AS BLOB)",
            "UPDATE episodic_summaries SET source_count=source_count+1",
            "UPDATE episodic_summaries SET plaintext_validation_version=plaintext_validation_version+1",
            "UPDATE episodic_summaries SET created_at_ms=created_at_ms+1",
            "UPDATE episodic_summaries SET creation_event_sequence=creation_event_sequence+1",
            "UPDATE episodic_summaries SET creation_event_id='00000000-0000-0000-0000-000000993005'",
            "UPDATE episodic_summaries SET source_set_digest='bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb'",
            "UPDATE episodic_summaries SET content_digest='cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc'",
            "UPDATE episodic_summaries SET record_digest='dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd'",
            "UPDATE episodic_summaries SET record_json=CAST('{}' AS BLOB)",
            "UPDATE episodic_summary_sources SET summary_id='00000000-0000-0000-0000-000000993006' WHERE source_ordinal=0",
            "UPDATE episodic_summary_sources SET source_ordinal=99 WHERE source_ordinal=0",
            "UPDATE episodic_summary_sources SET event_sequence=event_sequence+1 WHERE source_ordinal=0",
            "UPDATE episodic_summary_sources SET event_id='00000000-0000-0000-0000-000000993007' WHERE source_ordinal=0",
            "UPDATE episodic_summary_sources SET event_type='changed' WHERE source_ordinal=0",
            "UPDATE episodic_summary_sources SET event_digest='eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee' WHERE source_ordinal=0",
        ],
    );
}

fn row_count(database: &Database, table: &str) -> i64 {
    database
        .connection()
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn recovery_backfills_every_wholly_missing_event_proven_memory_row_without_artifacts() {
    let events = verified_memory_history();
    let (_temporary_directory, _paths, mut database) = event_only_database(&events);
    let event_count = row_count(&database, "event_stream");
    let receipt_count = row_count(&database, "command_receipts");
    let receipt_event_ref_count = row_count(&database, "command_event_refs");

    let state = ProjectionRepository::rebuild(database.connection_mut(), &events).unwrap();

    assert!(!state.memory.is_empty());
    assert_eq!(row_count(&database, "memory_entry_versions"), 4);
    assert_eq!(row_count(&database, "memory_proposals"), 5);
    assert_eq!(row_count(&database, "memory_proposal_resolutions"), 4);
    assert_eq!(
        database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM approval_records WHERE action_kind='memory_mutation'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        5
    );
    assert_eq!(row_count(&database, "episodic_summaries"), 1);
    assert_eq!(row_count(&database, "episodic_summary_sources"), 3);
    assert_eq!(row_count(&database, "current_memory_entries"), 3);
    assert_eq!(row_count(&database, "current_memory_proposal_status"), 5);
    assert_eq!(row_count(&database, "event_stream"), event_count);
    assert_eq!(row_count(&database, "command_receipts"), receipt_count);
    assert_eq!(
        row_count(&database, "command_event_refs"),
        receipt_event_ref_count
    );
}

fn memory_entry_object(entry: &MemoryEntryVersion) -> ObjectRef {
    let reference = entry.reference();
    ObjectRef::new(
        "memory_entry_version",
        reference.entry_version_id().to_string(),
        reference.version(),
        reference.content_digest().clone(),
    )
    .unwrap()
}

#[test]
fn recovery_backfills_predecessors_in_event_order_not_uuid_order() {
    let namespace = MemoryNamespaceId::from_uuid(Uuid::from_u128(995_001));
    let first = MemoryEntryVersion::create_present(
        namespace,
        MemoryEntryId::from_uuid(Uuid::from_u128(995_002)),
        MemoryEntryVersionId::from_uuid(Uuid::from_u128(u128::MAX - 1)),
        MemoryEntryDraft::new("Reverse IDs".into(), "first".into(), vec![]).unwrap(),
        Actor::Human,
        10,
        None,
        EventId::from_uuid(Uuid::from_u128(995_003)),
    )
    .unwrap();
    let second = first
        .next_present(
            MemoryEntryVersionId::from_uuid(Uuid::from_u128(1)),
            MemoryEntryDraft::new("Reverse IDs".into(), "second".into(), vec![]).unwrap(),
            Actor::Human,
            11,
            None,
            EventId::from_uuid(Uuid::from_u128(995_004)),
        )
        .unwrap();
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let mut source = Database::open(&paths).unwrap();
    let tx = source.immediate_transaction().unwrap();
    for (ordinal, entry) in [&first, &second].into_iter().enumerate() {
        EventRepository::append(
            &tx,
            ai_stock_forum::app::PendingEvent {
                event_id: entry.creation_event_id(),
                event_schema_version: ai_stock_forum::app::EVENT_SCHEMA_VERSION,
                actor: Actor::Human,
                occurred_at_ms: entry.created_at_ms(),
                correlation_id: CorrelationId::from_uuid(Uuid::from_u128(
                    995_100 + ordinal as u128,
                )),
                causation_id: None,
                object: Some(memory_entry_object(entry)),
                event: ai_stock_forum::app::ApplicationEvent::MemoryEntrySet {
                    entry: entry.clone(),
                    expired_proposals: vec![],
                },
            },
        )
        .unwrap();
    }
    tx.commit().unwrap();
    let events = EventRepository::load_all(source.connection()).unwrap();
    let (_target_directory, _target_paths, mut target) = event_only_database(&events);

    ProjectionRepository::rebuild(target.connection_mut(), &events).unwrap();

    assert_eq!(row_count(&target, "memory_entry_versions"), 2);
    let current: String = target
        .connection()
        .query_row(
            "SELECT entry_version_id FROM current_memory_entries",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(current, second.reference().entry_version_id().to_string());
}

fn pending_history(
    events: &[ai_stock_forum::app::EventEnvelope],
) -> Vec<ai_stock_forum::app::PendingEvent> {
    events
        .iter()
        .map(|event| ai_stock_forum::app::PendingEvent {
            event_id: event.event_id,
            event_schema_version: event.event_schema_version,
            actor: event.actor.clone(),
            occurred_at_ms: event.occurred_at_ms,
            correlation_id: event.correlation_id,
            causation_id: event.causation_id,
            object: event.object.clone(),
            event: event.event.clone(),
        })
        .collect()
}

fn assert_pending_history_rejected(pending: Vec<ai_stock_forum::app::PendingEvent>, case: &str) {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let mut database = Database::open(&paths).unwrap();
    let tx = database.immediate_transaction().unwrap();
    for event in pending {
        EventRepository::append(&tx, event).unwrap();
    }
    tx.commit().unwrap();
    let events = EventRepository::load_all(database.connection()).unwrap();
    let error = ProjectionRepository::rebuild(database.connection_mut(), &events).expect_err(case);
    assert_eq!(error.code(), "invalid_event_record", "{case}");
}

#[test]
fn recovery_authenticates_memory_event_envelopes_and_every_summary_source_ref() {
    let events = verified_memory_history();
    for (case, kind, mutation) in [
        ("direct entry actor", "memory_entry_set", "actor"),
        (
            "proposal requester actor",
            "memory_proposal_created",
            "actor",
        ),
        (
            "resolution occurrence time",
            "memory_proposal_accepted",
            "time",
        ),
        ("summary event identity", "episodic_summary_recorded", "id"),
    ] {
        let mut pending = pending_history(&events);
        let event = pending
            .iter_mut()
            .find(|event| event.event.kind() == kind)
            .unwrap();
        match mutation {
            "actor" => event.actor = Actor::System,
            "time" => event.occurred_at_ms += 1,
            "id" => event.event_id = EventId::from_uuid(Uuid::from_u128(996_001)),
            _ => unreachable!(),
        }
        assert_pending_history_rejected(pending, case);
    }

    let profile = events
        .iter()
        .find_map(|event| match &event.event {
            ai_stock_forum::app::ApplicationEvent::AgentProfileCreated { profile } => {
                Some(profile.clone())
            }
            _ => None,
        })
        .unwrap();
    let mut pending = pending_history(&events);
    let event = pending
        .iter_mut()
        .find(|event| event.event.kind() == "episodic_summary_recorded")
        .unwrap();
    let ai_stock_forum::app::ApplicationEvent::EpisodicSummaryRecorded { summary: original } =
        &event.event
    else {
        unreachable!()
    };
    let mut sources = original.sources().to_vec();
    let first = &sources[0];
    let wrong_digest =
        Digest::parse("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    assert_ne!(first.event_digest(), &wrong_digest);
    sources[0] = EpisodicSourceRef::new(
        first.sequence(),
        first.event_id(),
        first.event_type().to_owned(),
        wrong_digest,
    )
    .unwrap();
    let replacement = EpisodicSummary::new(
        original.reference().summary_id(),
        &profile,
        original.label().to_owned(),
        original.body().to_owned(),
        original.purpose_tags().to_vec(),
        sources,
        original.created_at_ms(),
        original.creation_event_sequence(),
        original.creation_event_id(),
    )
    .unwrap();
    let reference = replacement.reference();
    event.object = Some(
        ObjectRef::new(
            "episodic_summary",
            reference.summary_id().to_string(),
            reference.version(),
            reference.content_digest().clone(),
        )
        .unwrap(),
    );
    event.event = ai_stock_forum::app::ApplicationEvent::EpisodicSummaryRecorded {
        summary: replacement,
    };
    assert_pending_history_rejected(pending, "summary source digest substitution");
}

#[test]
fn recovery_repairs_each_wholly_missing_immutable_row_including_a_middle_source() {
    let events = verified_memory_history();
    let (_temporary_directory, _paths, mut database) = recovered_database(&events);
    database
        .connection()
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             DROP TRIGGER memory_entry_versions_no_delete;
             DROP TRIGGER memory_proposals_no_delete;
             DROP TRIGGER memory_proposal_resolutions_no_delete;
             DROP TRIGGER approval_records_no_delete;
             DROP TRIGGER episodic_summaries_no_delete;
             DROP TRIGGER episodic_summary_sources_no_delete;",
        )
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    let cases = [
        (
            "entry",
            "memory_entry_versions",
            "DELETE FROM memory_entry_versions WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
        ),
        (
            "proposal",
            "memory_proposals",
            "DELETE FROM memory_proposals WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
        ),
        (
            "resolution",
            "memory_proposal_resolutions",
            "DELETE FROM memory_proposal_resolutions WHERE rowid=(SELECT MIN(rowid) FROM memory_proposal_resolutions)",
        ),
        (
            "pending approval",
            "approval_records",
            "DELETE FROM approval_records WHERE rowid=(SELECT rowid FROM approval_records WHERE action_kind='memory_mutation' AND status='pending' LIMIT 1)",
        ),
        (
            "terminal approval",
            "approval_records",
            "DELETE FROM approval_records WHERE rowid=(SELECT rowid FROM approval_records WHERE action_kind='memory_mutation' AND status<>'pending' LIMIT 1)",
        ),
        (
            "summary",
            "episodic_summaries",
            "DELETE FROM episodic_summaries WHERE rowid=(SELECT MIN(rowid) FROM episodic_summaries)",
        ),
        (
            "middle source",
            "episodic_summary_sources",
            "DELETE FROM episodic_summary_sources WHERE source_ordinal=1",
        ),
    ];
    for (name, table, deletion) in cases {
        tx.transaction().execute_batch("SAVEPOINT missing").unwrap();
        let before = tx
            .transaction()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(tx.transaction().execute(deletion, []).unwrap(), 1, "{name}");
        ProjectionRepository::rebuild_in(&tx, &events).unwrap();
        let after = tx
            .transaction()
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(after, before, "{name}");
        tx.transaction()
            .execute_batch("ROLLBACK TO missing; RELEASE missing")
            .unwrap();
    }
    tx.rollback().unwrap();
}

#[test]
fn recovery_rejects_extra_immutable_memory_rows_but_preserves_unrelated_approvals() {
    let events = verified_memory_history();
    let (_temporary_directory, _paths, mut database) = recovered_database(&events);
    database
        .connection()
        .execute(
            "INSERT INTO approval_records (
                 approval_id,action_kind,object_kind,object_id,object_version,object_digest,
                 actor_kind,actor_id,status,created_at_ms,expires_at_ms,resolved_at_ms,
                 resolution_kind,resolution_event_id,resolution_actor_kind,resolution_actor_id
             ) VALUES (
                 'unrelated-approval','discussion_run','discussion','unrelated-object',1,
                 'unrelated-digest','human',NULL,'pending',42,NULL,NULL,NULL,NULL,NULL,NULL
             )",
            [],
        )
        .unwrap();
    let unrelated_before: String = database
        .connection()
        .query_row(
            "SELECT json_array(approval_id,action_kind,object_kind,object_id,object_version,
                 object_digest,actor_kind,actor_id,status,created_at_ms,expires_at_ms,
                 resolved_at_ms,resolution_kind,resolution_event_id,resolution_actor_kind,
                 resolution_actor_id)
             FROM approval_records WHERE approval_id='unrelated-approval'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    ProjectionRepository::rebuild(database.connection_mut(), &events).unwrap();
    let unrelated_after: String = database
        .connection()
        .query_row(
            "SELECT json_array(approval_id,action_kind,object_kind,object_id,object_version,
                 object_digest,actor_kind,actor_id,status,created_at_ms,expires_at_ms,
                 resolved_at_ms,resolution_kind,resolution_event_id,resolution_actor_kind,
                 resolution_actor_id)
             FROM approval_records WHERE approval_id='unrelated-approval'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(unrelated_after, unrelated_before);

    database
        .connection()
        .execute_batch(
            "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;
             DROP TRIGGER memory_entry_versions_identity_guard;
             DROP TRIGGER memory_entry_versions_predecessor_guard;
             DROP TRIGGER episodic_summary_sources_order_guard;",
        )
        .unwrap();
    let tx = database.immediate_transaction().unwrap();
    let extras = [
        (
            "entry",
            "INSERT INTO memory_entry_versions
             SELECT memory_namespace_id,'00000000-0000-0000-0000-000000994001',
                    '00000000-0000-0000-0000-000000994002',1,NULL,'Extra','extra',
                    state,value_text,value_bytes,purpose_tags_json,created_by_kind,created_by_id,
                    created_at_ms,accepted_proposal_id,accepted_proposal_version,
                    accepted_proposal_digest,plaintext_validation_version,creation_event_sequence,
                    creation_event_id,content_digest,record_digest,record_json
             FROM memory_entry_versions WHERE rowid=(SELECT MIN(rowid) FROM memory_entry_versions)",
        ),
        (
            "proposal",
            "INSERT INTO memory_proposals
             SELECT '00000000-0000-0000-0000-000000994003',version,proposer_profile_id,
                    proposer_profile_version_id,proposer_profile_version,proposer_profile_digest,
                    memory_namespace_id,operation,display_key,normalized_key,expected_kind,
                    expected_entry_id,expected_entry_version_id,expected_entry_version,
                    expected_entry_digest,candidate_value,candidate_value_bytes,
                    candidate_purpose_tags_json,rationale,plaintext_validation_version,
                    created_at_ms,creation_event_sequence,creation_event_id,
                    '00000000-0000-0000-0000-000000994004',content_digest,record_digest,record_json
             FROM memory_proposals WHERE rowid=(SELECT MIN(rowid) FROM memory_proposals)",
        ),
        (
            "resolution",
            "INSERT INTO memory_proposal_resolutions
             SELECT '00000000-0000-0000-0000-000000994005',proposal_version,
                    proposal_content_digest,status,'00000000-0000-0000-0000-000000994006',
                    resolved_by_kind,resolved_by_id,resolved_at_ms,resolution_event_sequence,
                    resolution_event_id,resolution_json
             FROM memory_proposal_resolutions
             WHERE rowid=(SELECT MIN(rowid) FROM memory_proposal_resolutions)",
        ),
        (
            "memory approval",
            "INSERT INTO approval_records
             SELECT '00000000-0000-0000-0000-000000994007',action_kind,object_kind,
                    '00000000-0000-0000-0000-000000994008',object_version,object_digest,
                    actor_kind,actor_id,status,created_at_ms,expires_at_ms,resolved_at_ms,
                    resolution_kind,resolution_event_id,resolution_actor_kind,resolution_actor_id
             FROM approval_records WHERE action_kind='memory_mutation'
             AND rowid=(SELECT MIN(rowid) FROM approval_records WHERE action_kind='memory_mutation')",
        ),
        (
            "summary",
            "INSERT INTO episodic_summaries
             SELECT '00000000-0000-0000-0000-000000994009',version,memory_namespace_id,
                    profile_id,profile_version_id,profile_version,profile_content_digest,label,
                    body,purpose_tags_json,source_count,plaintext_validation_version,created_at_ms,
                    999999,'00000000-0000-0000-0000-000000994010',source_set_digest,
                    content_digest,record_digest,record_json FROM episodic_summaries",
        ),
        (
            "source",
            "INSERT INTO episodic_summary_sources
             SELECT s.summary_id,99,e.sequence,e.event_id,e.event_type,e.event_digest
             FROM episodic_summaries s JOIN event_stream e
             WHERE e.sequence<s.creation_event_sequence
               AND NOT EXISTS (
                   SELECT 1 FROM episodic_summary_sources existing
                   WHERE existing.summary_id=s.summary_id AND existing.event_id=e.event_id
               )
             ORDER BY e.sequence LIMIT 1",
        ),
    ];
    for (name, insertion) in extras {
        tx.transaction().execute_batch("SAVEPOINT extra").unwrap();
        assert_eq!(
            tx.transaction().execute(insertion, []).unwrap(),
            1,
            "{name}"
        );
        let error = ProjectionRepository::rebuild_in(&tx, &events).expect_err(name);
        assert_eq!(error.code(), "invalid_event_record", "{name}");
        tx.transaction()
            .execute_batch("ROLLBACK TO extra; RELEASE extra")
            .unwrap();
    }
    tx.rollback().unwrap();
}

fn query_snapshot(database: &Database, query: &str) -> Vec<String> {
    let mut statement = database.connection().prepare(query).unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn durable_memory_snapshot(database: &Database) -> Vec<Vec<String>> {
    [
        "SELECT quote(memory_namespace_id)||'|'||quote(entry_id)||'|'||quote(entry_version_id)||'|'||quote(version)||'|'||quote(predecessor_version_id)||'|'||quote(display_key)||'|'||quote(normalized_key)||'|'||quote(state)||'|'||quote(value_text)||'|'||quote(value_bytes)||'|'||quote(purpose_tags_json)||'|'||quote(created_by_kind)||'|'||quote(created_by_id)||'|'||quote(created_at_ms)||'|'||quote(accepted_proposal_id)||'|'||quote(accepted_proposal_version)||'|'||quote(accepted_proposal_digest)||'|'||quote(plaintext_validation_version)||'|'||quote(creation_event_sequence)||'|'||quote(creation_event_id)||'|'||quote(content_digest)||'|'||quote(record_digest)||'|'||quote(record_json) FROM memory_entry_versions ORDER BY entry_id,version",
        "SELECT quote(proposal_id)||'|'||quote(version)||'|'||quote(proposer_profile_id)||'|'||quote(proposer_profile_version_id)||'|'||quote(proposer_profile_version)||'|'||quote(proposer_profile_digest)||'|'||quote(memory_namespace_id)||'|'||quote(operation)||'|'||quote(display_key)||'|'||quote(normalized_key)||'|'||quote(expected_kind)||'|'||quote(expected_entry_id)||'|'||quote(expected_entry_version_id)||'|'||quote(expected_entry_version)||'|'||quote(expected_entry_digest)||'|'||quote(candidate_value)||'|'||quote(candidate_value_bytes)||'|'||quote(candidate_purpose_tags_json)||'|'||quote(rationale)||'|'||quote(plaintext_validation_version)||'|'||quote(created_at_ms)||'|'||quote(creation_event_sequence)||'|'||quote(creation_event_id)||'|'||quote(approval_id)||'|'||quote(content_digest)||'|'||quote(record_digest)||'|'||quote(record_json) FROM memory_proposals ORDER BY proposal_id",
        "SELECT quote(proposal_id)||'|'||quote(proposal_version)||'|'||quote(proposal_content_digest)||'|'||quote(status)||'|'||quote(approval_id)||'|'||quote(resolved_by_kind)||'|'||quote(resolved_by_id)||'|'||quote(resolved_at_ms)||'|'||quote(resolution_event_sequence)||'|'||quote(resolution_event_id)||'|'||quote(resolution_json) FROM memory_proposal_resolutions ORDER BY proposal_id",
        "SELECT quote(summary_id)||'|'||quote(version)||'|'||quote(memory_namespace_id)||'|'||quote(profile_id)||'|'||quote(profile_version_id)||'|'||quote(profile_version)||'|'||quote(profile_content_digest)||'|'||quote(label)||'|'||quote(body)||'|'||quote(purpose_tags_json)||'|'||quote(source_count)||'|'||quote(plaintext_validation_version)||'|'||quote(created_at_ms)||'|'||quote(creation_event_sequence)||'|'||quote(creation_event_id)||'|'||quote(source_set_digest)||'|'||quote(content_digest)||'|'||quote(record_digest)||'|'||quote(record_json) FROM episodic_summaries ORDER BY summary_id",
        "SELECT quote(summary_id)||'|'||quote(source_ordinal)||'|'||quote(event_sequence)||'|'||quote(event_id)||'|'||quote(event_type)||'|'||quote(event_digest) FROM episodic_summary_sources ORDER BY summary_id,source_ordinal",
        "SELECT quote(approval_id)||'|'||quote(action_kind)||'|'||quote(object_kind)||'|'||quote(object_id)||'|'||quote(object_version)||'|'||quote(object_digest)||'|'||quote(actor_kind)||'|'||quote(actor_id)||'|'||quote(status)||'|'||quote(created_at_ms)||'|'||quote(expires_at_ms)||'|'||quote(resolved_at_ms)||'|'||quote(resolution_kind)||'|'||quote(resolution_event_id)||'|'||quote(resolution_actor_kind)||'|'||quote(resolution_actor_id) FROM approval_records ORDER BY approval_id",
        "SELECT quote(memory_namespace_id)||'|'||quote(normalized_key)||'|'||quote(entry_id)||'|'||quote(entry_version_id)||'|'||quote(version)||'|'||quote(state)||'|'||quote(content_digest) FROM current_memory_entries ORDER BY memory_namespace_id,normalized_key",
        "SELECT quote(proposal_id)||'|'||quote(proposal_version)||'|'||quote(proposal_content_digest)||'|'||quote(memory_namespace_id)||'|'||quote(normalized_key)||'|'||quote(status)||'|'||quote(resolution_event_id)||'|'||quote(created_at_ms) FROM current_memory_proposal_status ORDER BY proposal_id",
        "SELECT quote(last_event_sequence)||'|'||quote(last_event_digest)||'|'||quote(projection_digest) FROM projection_metadata ORDER BY singleton",
    ]
    .iter()
    .map(|query| query_snapshot(database, query))
    .collect()
}

#[test]
fn failed_reconciliation_is_atomic_and_repeated_recovery_is_byte_identical() {
    let events = verified_memory_history();
    let (_temporary_directory, _paths, mut database) = recovered_database(&events);
    let before = durable_memory_snapshot(&database);
    let before_projection = ProjectionRepository::load(database.connection()).unwrap();
    let event_count = row_count(&database, "event_stream");
    let receipt_count = row_count(&database, "command_receipts");

    let first = ProjectionRepository::rebuild(database.connection_mut(), &events).unwrap();
    assert_eq!(first, before_projection);
    assert_eq!(durable_memory_snapshot(&database), before);
    let second = ProjectionRepository::rebuild(database.connection_mut(), &events).unwrap();
    assert_eq!(second, before_projection);
    assert_eq!(durable_memory_snapshot(&database), before);
    assert_eq!(row_count(&database, "event_stream"), event_count);
    assert_eq!(row_count(&database, "command_receipts"), receipt_count);

    database
        .connection()
        .execute_batch(
            "PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;
             DROP TRIGGER approval_records_no_delete;
             DROP TRIGGER episodic_summaries_no_update;",
        )
        .unwrap();
    let missing_approval: String = database
        .connection()
        .query_row(
            "SELECT approval_id FROM approval_records WHERE action_kind='memory_mutation' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    database
        .connection()
        .execute(
            "DELETE FROM approval_records WHERE approval_id=?1",
            [&missing_approval],
        )
        .unwrap();
    database
        .connection()
        .execute(
            "UPDATE episodic_summaries SET label='corrupt private label'",
            [],
        )
        .unwrap();
    let error = ProjectionRepository::rebuild(database.connection_mut(), &events).unwrap_err();
    assert_eq!(error.code(), "invalid_event_record");
    assert_eq!(
        database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM approval_records WHERE approval_id=?1",
                [&missing_approval],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0,
        "the early approval backfill must roll back after a later mismatch"
    );
    let stored_label: String = database
        .connection()
        .query_row("SELECT label FROM episodic_summaries", [], |row| row.get(0))
        .unwrap();
    assert_eq!(stored_label, "corrupt private label");
}

#[test]
fn legacy_event_stream_rebuild_keeps_the_compatible_empty_memory_projection() {
    let mut fixture = support::database();
    fixture.append(ai_stock_forum::app::ApplicationEvent::HelpViewed);
    let events = EventRepository::load_all(fixture.database.connection()).unwrap();
    let mut expected = ProjectionState::default();
    for event in &events {
        reduce(&mut expected, event).unwrap();
    }

    let rebuilt =
        ProjectionRepository::rebuild(fixture.database.connection_mut(), &events).unwrap();

    assert_eq!(rebuilt, expected);
    assert!(rebuilt.memory.is_empty());
    assert_eq!(rebuilt.digest().unwrap(), expected.digest().unwrap());
    let canonical = String::from_utf8(canonical_json_bytes(&rebuilt).unwrap()).unwrap();
    assert!(!canonical.contains("\"memory\""));
    for table in [
        "memory_entry_versions",
        "memory_proposals",
        "memory_proposal_resolutions",
        "episodic_summaries",
        "episodic_summary_sources",
        "current_memory_entries",
        "current_memory_proposal_status",
    ] {
        assert_eq!(row_count(&fixture.database, table), 0, "{table}");
    }
}

#[test]
fn snapshot_receipt_replay_after_restart_uses_recovered_historical_rows_without_new_work() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let fixture = support::persistent_fixture();
    let profile = persistent_profile(&fixture);
    let mut app = fixture.service_with_policy(Arc::new(policy.clone()));

    let first_review = match app
        .preview_memory_set(
            AgentProfileSelector::from(profile.profile_id()),
            MemoryEntryDraft::new(
                "Receipt recovery key".into(),
                "historical recovered plaintext".into(),
                vec![],
            )
            .unwrap(),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected first memory review, got {other:?}"),
    };
    app.execute(envelope(
        400,
        Actor::Human,
        ApplicationCommand::SetMemoryEntry {
            profile: first_review.profile,
            expected: first_review.expected,
            candidate: first_review.candidate.unwrap(),
            review_token: first_review.review_token,
            review_digest: first_review.review_digest,
        },
    ))
    .unwrap();
    let request = MemoryRetrievalRequest::new(
        MemoryRetrievalScope::new(&profile, MemoryPurposeScope::General).unwrap(),
        MemoryRetrievalBudget::new(1, 0, 32_768, 0).unwrap(),
    )
    .unwrap();
    let command = envelope(
        401,
        Actor::Human,
        ApplicationCommand::BuildMemorySnapshot { request },
    );
    let historical = app.execute(command.clone()).unwrap();
    let CommandView::MemorySnapshot(historical_view) = &historical.view else {
        panic!("expected memory snapshot");
    };
    assert_eq!(historical_view.snapshot.entries().len(), 1);
    assert_eq!(
        historical_view.snapshot.entries()[0].value(),
        "historical recovered plaintext"
    );

    let successor_review = match app
        .preview_memory_set(
            AgentProfileSelector::from(profile.profile_id()),
            MemoryEntryDraft::new(
                "Receipt recovery key".into(),
                "new live plaintext".into(),
                vec![],
            )
            .unwrap(),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected successor memory review, got {other:?}"),
    };
    app.execute(envelope(
        402,
        Actor::Human,
        ApplicationCommand::SetMemoryEntry {
            profile: successor_review.profile,
            expected: successor_review.expected,
            candidate: successor_review.candidate.unwrap(),
            review_token: successor_review.review_token,
            review_digest: successor_review.review_digest,
        },
    ))
    .unwrap();
    app.finish(ShutdownReason::UserQuit).unwrap();
    drop(app);

    let database = fixture.open_database();
    database
        .connection()
        .execute_batch(
            "PRAGMA foreign_keys=OFF;
             DROP TRIGGER memory_entry_versions_no_delete;
             DELETE FROM current_memory_entries;
             DELETE FROM memory_entry_versions;",
        )
        .unwrap();
    drop(database);

    policy.set_decision(AuthorizationDecision::Denied(
        ai_stock_forum::policy::PolicyDecision::Denied,
    ));
    let mut restarted = fixture.service_with_policy(Arc::new(policy.clone()));
    assert_eq!(fixture.count_rows("memory_entry_versions"), 2);
    let database = fixture.open_database();
    database
        .connection()
        .execute_batch("DELETE FROM current_memory_entries; DELETE FROM active_agent_profiles;")
        .unwrap();
    drop(database);
    let side_effects = fixture.side_effect_calls();
    let policy_calls = policy.calls();
    let event_count = fixture.count_rows("event_stream");
    let receipt_count = fixture.count_rows("command_receipts");

    let replay = restarted.execute(command).unwrap();

    assert_eq!(replay, historical);
    assert_eq!(fixture.side_effect_calls(), side_effects);
    assert_eq!(policy.calls(), policy_calls);
    assert_eq!(fixture.count_rows("event_stream"), event_count);
    assert_eq!(fixture.count_rows("command_receipts"), receipt_count);
}

#[test]
fn startup_replaces_corrupt_rebuildable_memory_pointers_and_statuses() {
    let events = verified_memory_history();
    let (_temporary_directory, paths, database) = recovered_database(&events);
    let expected = current_projection_rows(&database);
    let terminal_event = events
        .iter()
        .find(|event| event.event.kind() == "memory_proposal_accepted")
        .unwrap()
        .event_id;
    database
        .connection()
        .execute_batch("PRAGMA foreign_keys=OFF; PRAGMA ignore_check_constraints=ON;")
        .unwrap();
    database
        .connection()
        .execute(
            "UPDATE current_memory_entries SET normalized_key='corrupt-key' WHERE rowid=(SELECT MIN(rowid) FROM current_memory_entries)",
            [],
        )
        .unwrap();
    database
        .connection()
        .execute(
            "UPDATE current_memory_proposal_status SET status='accepted', resolution_event_id=?1
             WHERE proposal_id=(SELECT proposal_id FROM current_memory_proposal_status WHERE status='pending' LIMIT 1)",
            [terminal_event.to_string()],
        )
        .unwrap();
    drop(database);

    let clock = Arc::new(support::TestClock::new());
    let ids = Arc::new(support::TestIds::new());
    for _ in 0..1_000 {
        ids.next_uuid();
    }
    let mut app = ApplicationService::bootstrap(&paths, clock, ids).unwrap();
    app.finish(ShutdownReason::UserQuit).unwrap();
    drop(app);

    let database = Database::open(&paths).unwrap();
    assert_eq!(current_projection_rows(&database), expected);
}

fn envelope(
    id: u128,
    actor: Actor,
    command: ApplicationCommand,
) -> ai_stock_forum::app::CommandEnvelope {
    ai_stock_forum::app::CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 20_000)),
        actor,
        command,
    }
}

#[test]
fn startup_rebuilds_a_missing_current_memory_pointer_from_verified_events() {
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let clock = Arc::new(support::TestClock::new());
    let ids = Arc::new(support::TestIds::new());
    let mut app =
        ai_stock_forum::app::ApplicationService::bootstrap(&paths, clock.clone(), ids.clone())
            .unwrap();
    let created = app
        .execute_user(ApplicationCommand::CreateAgentProfile {
            draft: AgentProfileDraft::new(
                "Recovery memory".into(),
                "Recovery contract profile.".into(),
                AgentRole::Custom,
                "research".into(),
                vec![],
                "Careful.".into(),
                "Use evidence.".into(),
                AgentBindings::default(),
                vec![],
                vec![],
            )
            .unwrap(),
            template_provenance: None,
        })
        .unwrap();
    let CommandView::AgentProfileCreated(created) = created.view else {
        panic!("expected profile creation");
    };
    let review = match app
        .preview_memory_set(
            AgentProfileSelector::from(created.profile_id),
            MemoryEntryDraft::new("recovery key".into(), "historical value".into(), vec![])
                .unwrap(),
        )
        .unwrap()
    {
        MemoryEditPreview::Review(review) => review,
        other => panic!("expected memory review, got {other:?}"),
    };
    app.execute_user(ApplicationCommand::SetMemoryEntry {
        profile: review.profile,
        expected: review.expected,
        candidate: review.candidate.unwrap(),
        review_token: review.review_token,
        review_digest: review.review_digest,
    })
    .unwrap();
    app.finish(ShutdownReason::UserQuit).unwrap();
    drop(app);

    let mut database = Database::open(&paths).unwrap();
    database
        .connection_mut()
        .execute("DELETE FROM current_memory_entries", [])
        .unwrap();
    drop(database);

    let restarted = ai_stock_forum::app::ApplicationService::bootstrap(&paths, clock, ids).unwrap();
    drop(restarted);
    let database = Database::open(&paths).unwrap();
    let pointers: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM current_memory_entries", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        pointers, 1,
        "verified history must rebuild the missing pointer"
    );
}

#[test]
fn rebuilds_missing_terminal_proposal_resolution_and_approval_from_verified_events() {
    let mut fixture = support::persistent_fixture();
    let mut app = fixture.service();
    let created = app
        .execute(envelope(
            81,
            Actor::Human,
            ApplicationCommand::CreateAgentProfile {
                draft: AgentProfileDraft::new(
                    "Recovery proposal".into(),
                    "Recovery contract profile.".into(),
                    AgentRole::Custom,
                    "research".into(),
                    vec![],
                    "Careful.".into(),
                    "Use evidence.".into(),
                    AgentBindings::default(),
                    vec![],
                    vec![],
                )
                .unwrap(),
                template_provenance: None,
            },
        ))
        .unwrap();
    let CommandView::AgentProfileCreated(created) = created.view else {
        panic!("expected profile creation");
    };
    let profile = fixture.active_profile(created.profile_id);
    let created = app
        .execute(envelope(
            82,
            Actor::Agent(profile.profile_id()),
            ApplicationCommand::ProposeMemoryMutation {
                proposer: profile.reference(),
                expected: ai_stock_forum::memory::ExpectedMemoryEntryState::Absent,
                operation: ai_stock_forum::memory::MemoryProposalOperation::Set {
                    candidate: MemoryEntryDraft::new(
                        "recovery proposal".into(),
                        "value".into(),
                        vec![],
                    )
                    .unwrap(),
                },
                rationale: "Verified recovery proposal.".into(),
            },
        ))
        .unwrap();
    let proposal = match created.view {
        CommandView::MemoryProposalCreated(view) => view.proposal,
        other => panic!("expected proposal creation, got {other:?}"),
    };
    let review = app
        .preview_memory_proposal_approval(proposal.clone())
        .unwrap();
    app.execute(envelope(
        83,
        Actor::Human,
        ApplicationCommand::ApproveMemoryProposal {
            proposal,
            approval_id: review.approval_id,
            expected_approval_status: review.expected_approval_status,
            expected_entry: review.expected_entry,
            review_token: review.review_token,
            review_digest: review.review_digest,
        },
    ))
    .unwrap();
    app.finish(ShutdownReason::UserQuit).unwrap();
    drop(app);
    let source_event_id = fixture
        .events()
        .into_iter()
        .find(|event| event.kind == "agent_profile_created")
        .unwrap()
        .event_id;
    support::record_test_episodic_summary(
        &mut fixture,
        profile.reference(),
        "Recovery summary".into(),
        "Verified episodic history.".into(),
        Vec::new(),
        vec![source_event_id],
    )
    .unwrap();
    let source = fixture.open_database();
    let events = EventRepository::load_all(source.connection()).unwrap();
    let temporary_directory = tempfile::tempdir().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let mut database = Database::open(&paths).unwrap();
    let tx = database.immediate_transaction().unwrap();
    for event in &events {
        EventRepository::append(
            &tx,
            ai_stock_forum::app::PendingEvent {
                event_id: event.event_id,
                event_schema_version: event.event_schema_version,
                actor: event.actor.clone(),
                occurred_at_ms: event.occurred_at_ms,
                correlation_id: event.correlation_id,
                causation_id: event.causation_id,
                object: event.object.clone(),
                event: event.event.clone(),
            },
        )
        .unwrap();
    }
    tx.commit().unwrap();
    ProjectionRepository::rebuild(database.connection_mut(), &events).unwrap();
    let proposals: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM memory_proposals", [], |row| {
            row.get(0)
        })
        .unwrap();
    let approvals: i64 = database
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM approval_records WHERE action_kind='memory_mutation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(proposals, 1);
    assert_eq!(approvals, 1);
    let resolutions: i64 = database
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM memory_proposal_resolutions",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(resolutions, 1);
    let summaries: i64 = database
        .connection()
        .query_row("SELECT COUNT(*) FROM episodic_summaries", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(summaries, 1);
}

#[test]
fn test_episodic_seeder_commits_one_exact_event_and_immutable_source_set() {
    let mut fixture = support::persistent_fixture();
    let profile = persistent_profile(&fixture);
    let source = fixture
        .events()
        .into_iter()
        .find(|event| event.kind == "agent_profile_created")
        .unwrap();
    let calls_before = fixture.side_effect_calls();
    let events_before = fixture.count_rows("event_stream");

    let summary_ref = support::record_test_episodic_summary(
        &mut fixture,
        profile.reference(),
        "  Recovery summary  ".into(),
        "Line one.\r\nLine two.".into(),
        vec![" Beta ".into(), "alpha".into()],
        vec![source.event_id],
    )
    .unwrap();

    assert_eq!(
        fixture.side_effect_calls(),
        (calls_before.0 + 2, calls_before.1 + 1)
    );
    assert_eq!(fixture.count_rows("event_stream"), events_before + 1);
    assert_eq!(fixture.count_rows("episodic_summaries"), 1);
    assert_eq!(fixture.count_rows("episodic_summary_sources"), 1);
    let event = fixture.events().into_iter().last().unwrap();
    assert_eq!(event.kind, "episodic_summary_recorded");
    assert_eq!(event.event_id, summary_ref.creation_event_id());
    assert_eq!(event.sequence, summary_ref.creation_event_sequence());
    let mut database = fixture.open_database();
    let tx = database.immediate_transaction().unwrap();
    let loaded = MemoryRepository::load_episodic_summary(&tx, summary_ref.summary_id())
        .unwrap()
        .unwrap();
    assert_eq!(loaded.reference(), summary_ref);
    assert_eq!(loaded.label(), "Recovery summary");
    assert_eq!(loaded.body(), "Line one.\nLine two.");
    assert_eq!(loaded.purpose_tags(), ["alpha", "Beta"]);
    let source_event = EventRepository::load_all(tx.transaction())
        .unwrap()
        .into_iter()
        .find(|candidate| candidate.event_id == source.event_id)
        .unwrap();
    assert_eq!(loaded.sources()[0].sequence(), source_event.sequence);
    assert_eq!(loaded.sources()[0].event_id(), source_event.event_id);
    assert_eq!(loaded.sources()[0].event_type(), source_event.event.kind());
    assert_eq!(
        loaded.sources()[0].event_digest().as_str(),
        source_event.event_digest.as_str()
    );
    tx.rollback().unwrap();
}

#[test]
fn test_episodic_seeder_rejects_every_invalid_input_without_ids_clock_or_writes() {
    let mut fixture = support::persistent_fixture();
    let profile = persistent_profile(&fixture);
    let events = fixture.events();
    let source_ids = events
        .iter()
        .filter(|event| event.kind != "episodic_summary_recorded")
        .map(|event| event.event_id)
        .take(2)
        .collect::<Vec<_>>();
    assert_eq!(source_ids.len(), 2);
    let wrong_profile = ai_stock_forum::agents::AgentProfileVersionRef::new(
        profile.profile_id(),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(900_001)),
        profile.version(),
        profile.content_digest().clone(),
    )
    .unwrap();
    let missing_source = EventId::from_uuid(Uuid::from_u128(900_002));
    let cases = [
        (
            "credential label",
            profile.reference(),
            "sk-abcdefghijklmnopqrst".to_owned(),
            "valid body".to_owned(),
            Vec::new(),
            vec![source_ids[0]],
            "unsafe_memory_text",
        ),
        (
            "credential body",
            profile.reference(),
            "valid label".to_owned(),
            "-----BEGIN PRIVATE KEY-----".to_owned(),
            Vec::new(),
            vec![source_ids[0]],
            "unsafe_memory_text",
        ),
        (
            "reserved tag",
            profile.reference(),
            "valid label".to_owned(),
            "valid body".to_owned(),
            vec!["general".to_owned()],
            vec![source_ids[0]],
            "invalid_memory_field",
        ),
        (
            "empty sources",
            profile.reference(),
            "valid label".to_owned(),
            "valid body".to_owned(),
            Vec::new(),
            Vec::new(),
            "invalid_episodic_summary",
        ),
        (
            "missing source",
            profile.reference(),
            "valid label".to_owned(),
            "valid body".to_owned(),
            Vec::new(),
            vec![missing_source],
            "invalid_event_record",
        ),
        (
            "profile substitution",
            wrong_profile,
            "valid label".to_owned(),
            "valid body".to_owned(),
            Vec::new(),
            vec![source_ids[0]],
            "unknown_profile",
        ),
        (
            "duplicate sources",
            profile.reference(),
            "valid label".to_owned(),
            "valid body".to_owned(),
            Vec::new(),
            vec![source_ids[0], source_ids[0]],
            "episodic_sources_not_ordered",
        ),
        (
            "reordered sources",
            profile.reference(),
            "valid label".to_owned(),
            "valid body".to_owned(),
            Vec::new(),
            vec![source_ids[1], source_ids[0]],
            "episodic_sources_not_ordered",
        ),
    ];

    for (name, profile, label, body, tags, sources, expected_code) in cases {
        let calls_before = fixture.side_effect_calls();
        let events_before = fixture.count_rows("event_stream");
        let error = support::record_test_episodic_summary(
            &mut fixture,
            profile,
            label,
            body,
            tags,
            sources,
        )
        .unwrap_err();
        assert_eq!(error.code(), expected_code, "{name}");
        assert_eq!(fixture.side_effect_calls(), calls_before, "{name}");
        assert_eq!(fixture.count_rows("event_stream"), events_before, "{name}");
        assert_eq!(fixture.count_rows("episodic_summaries"), 0, "{name}");
        assert_eq!(fixture.count_rows("episodic_summary_sources"), 0, "{name}");
    }
}

#[test]
fn test_episodic_seeder_validates_plaintext_before_requesting_a_write_transaction() {
    let mut fixture = support::persistent_fixture();
    let profile = persistent_profile(&fixture);
    let source_id = fixture.events()[0].event_id;
    let mut blocker = fixture.open_database();
    let _write_transaction = blocker.immediate_transaction().unwrap();
    let calls_before = fixture.side_effect_calls();

    let error = support::record_test_episodic_summary(
        &mut fixture,
        profile.reference(),
        "sk-abcdefghijklmnopqrst".into(),
        "valid body".into(),
        Vec::new(),
        vec![source_id],
    )
    .unwrap_err();

    assert_eq!(error.code(), "unsafe_memory_text");
    assert_eq!(fixture.side_effect_calls(), calls_before);
}
