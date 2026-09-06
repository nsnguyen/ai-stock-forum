mod support;

use std::thread;

use ai_stock_forum::{
    app::{AppError, ApplicationCommand, CommandEnvelope, CommandView},
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
        "Concurrent Skill".to_owned(),
        "Concurrency behavior.".to_owned(),
        "Use for concurrency tests.".to_owned(),
        Vec::new(),
        instructions.to_owned(),
        Vec::new(),
    )
    .unwrap()
}

#[test]
fn concurrent_versions_against_one_active_version_have_one_winner_and_one_stale_loser() {
    let mut app = support::app();
    let create_preview = app.preview_skill_creation(draft("Version one.")).unwrap();
    let created = app
        .execute(envelope(
            50_000,
            ApplicationCommand::CreateSkill {
                skill_id: create_preview.skill_id,
                candidate: draft("Version one."),
                review_token: create_preview.review_token,
                review_digest: create_preview.review_digest,
            },
        ))
        .unwrap();
    let CommandView::SkillCreated(created) = created.view else {
        panic!("created")
    };
    let mut first = app.independent_skill_instance().unwrap();
    let mut second = app.independent_skill_instance().unwrap();
    let first_candidate = draft("First version two.");
    let second_candidate = draft("Second version two.");
    let first_preview = first
        .preview_skill_version(
            created.skill_id,
            created.skill_version_id,
            first_candidate.clone(),
        )
        .unwrap();
    let second_preview = second
        .preview_skill_version(
            created.skill_id,
            created.skill_version_id,
            second_candidate.clone(),
        )
        .unwrap();

    let first_command = envelope(
        50_001,
        ApplicationCommand::ActivateSkillVersion {
            skill_id: created.skill_id,
            expected_active_version_id: created.skill_version_id,
            candidate: first_candidate,
            review_token: first_preview.review_token,
            review_digest: first_preview.review_digest,
        },
    );
    let second_command = envelope(
        50_002,
        ApplicationCommand::ActivateSkillVersion {
            skill_id: created.skill_id,
            expected_active_version_id: created.skill_version_id,
            candidate: second_candidate,
            review_token: second_preview.review_token,
            review_digest: second_preview.review_digest,
        },
    );
    let first_thread = thread::spawn(move || first.execute(first_command));
    let second_thread = thread::spawn(move || second.execute(second_command));
    let results = [first_thread.join().unwrap(), second_thread.join().unwrap()];

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(AppError::StaleSkillVersion)))
            .count(),
        1,
    );
    assert_eq!(app.event_count("skill_version_activated"), 1);
}
