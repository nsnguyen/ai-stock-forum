mod support;

use ai_stock_forum::{
    app::{ApplicationCommand, CommandEnvelope, CommandView},
    domain::{Actor, CommandId, CorrelationId},
    skills::SkillDraft,
};
use uuid::Uuid;

fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 1_000)),
        actor: Actor::Human,
        command,
    }
}

fn draft(instructions: &str) -> SkillDraft {
    SkillDraft::new(
        "Receipt Skill".to_owned(),
        "Receipt behavior.".to_owned(),
        "Use for receipt tests.".to_owned(),
        Vec::new(),
        instructions.to_owned(),
        Vec::new(),
    )
    .unwrap()
}

#[test]
fn skill_mutation_receipts_replay_the_original_typed_outcome_without_duplicate_writes() {
    let mut app = support::app();
    let preview = app.preview_skill_creation(draft("Version one.")).unwrap();
    let command = envelope(
        30_000,
        ApplicationCommand::CreateSkill {
            skill_id: preview.skill_id,
            candidate: draft("Version one."),
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
    );
    let before_events = app.count_rows("event_stream");
    let before_receipts = app.count_rows("command_receipts");
    let first = app.execute(command.clone()).unwrap();
    let after_first_events = app.count_rows("event_stream");
    let after_first_receipts = app.count_rows("command_receipts");
    let replay = app.execute(command).unwrap();

    assert_eq!(replay, first);
    assert!(matches!(first.view, CommandView::SkillCreated(_)));
    assert_eq!(after_first_events, before_events + 1);
    assert_eq!(after_first_receipts, before_receipts + 1);
    assert_eq!(app.count_rows("event_stream"), after_first_events);
    assert_eq!(app.count_rows("command_receipts"), after_first_receipts);
}

#[test]
fn a_replayed_command_id_with_a_changed_skill_candidate_is_a_stable_conflict() {
    let mut app = support::app();
    let preview = app.preview_skill_creation(draft("Version one.")).unwrap();
    let command = envelope(
        31_000,
        ApplicationCommand::CreateSkill {
            skill_id: preview.skill_id,
            candidate: draft("Version one."),
            review_token: preview.review_token,
            review_digest: preview.review_digest,
        },
    );
    app.execute(command.clone()).unwrap();
    let mut changed = command;
    let ApplicationCommand::CreateSkill { candidate, .. } = &mut changed.command else {
        panic!("create skill")
    };
    *candidate = draft("Changed after commit.");

    let error = app.execute(changed).unwrap_err();
    assert_eq!(error.code(), "command_conflict");
}
