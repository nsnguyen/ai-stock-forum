mod support;

use std::{io::Cursor, sync::Arc};

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentRole},
    app::{
        AppError, ApplicationCommand, ApplicationService, CommandView, InputRejectionCategory,
        ShutdownReason, SkillSelector,
    },
    config::AppPaths,
    domain::ObjectVersion,
    runtime::{ApplicationRuntime, RuntimeClient, RuntimeError},
    skills::SkillDraft,
    ui::command::{FallbackParsedLine, FallbackRunner, parse_fallback_line},
};
use tempfile::TempDir;

fn direct_command(input: &[u8]) -> ApplicationCommand {
    match parse_fallback_line(input) {
        FallbackParsedLine::Command(command) => command,
        FallbackParsedLine::AgentWorkflow(_) => panic!("expected direct command"),
        FallbackParsedLine::SkillWorkflow(_) => panic!("expected direct command"),
        FallbackParsedLine::Ignored => panic!("expected command"),
    }
}

fn profile(name: &str) -> AgentProfileDraft {
    AgentProfileDraft::new(
        name.to_owned(),
        "Fallback skill workflow profile.".to_owned(),
        AgentRole::Custom,
        "fallback review".to_owned(),
        vec!["fallback".to_owned()],
        "Deliberate.".to_owned(),
        "Keep exact skill references.".to_owned(),
        AgentBindings::default(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap()
}

fn skill(name: &str, instructions: &str) -> SkillDraft {
    SkillDraft::new(
        name.to_owned(),
        "Fallback skill workflow.".to_owned(),
        "Use while testing fallback review.".to_owned(),
        vec!["fallback".to_owned()],
        instructions.to_owned(),
        Vec::new(),
    )
    .unwrap()
}

fn create_profile(client: &RuntimeClient, name: &str) {
    let outcome = client
        .submit(ApplicationCommand::CreateAgentProfile {
            draft: profile(name),
            template_provenance: None,
        })
        .unwrap();
    assert!(matches!(outcome.view, CommandView::AgentProfileCreated(_)));
}

#[test]
fn parses_read_commands_with_quoted_selectors_and_positive_exact_versions() {
    assert_eq!(
        direct_command(b"/skill list"),
        ApplicationCommand::ListSkills
    );
    assert_eq!(direct_command(b"/skills"), ApplicationCommand::ListSkills);
    assert_eq!(
        direct_command(b"/skill show \"Evidence Review\""),
        ApplicationCommand::ShowSkill {
            selector: SkillSelector::from_input("Evidence Review").unwrap(),
        }
    );
    assert_eq!(
        direct_command(b"/skill show \"Evidence Review\" 1"),
        ApplicationCommand::ShowSkillVersion {
            selector: SkillSelector::from_input("Evidence Review").unwrap(),
            version: ObjectVersion::new(1).unwrap(),
        }
    );
}

#[test]
fn malformed_and_ambiguous_skill_commands_are_typed_and_actionable() {
    for input in [
        b"/skill".as_slice(),
        b"/skill show",
        b"/skill show evidence 0",
        b"/skill show evidence nope",
        b"/skill assign evidence agent 0",
        b"/skill assign evidence",
        b"/skill unassign evidence agent 1",
        b"/skill unknown",
        b"/skill show \"unterminated",
    ] {
        let ApplicationCommand::RejectInput(rejection) = direct_command(input) else {
            panic!("expected typed rejection for {input:?}");
        };
        assert_eq!(rejection.category, InputRejectionCategory::Malformed);
    }

    let runtime = support::runtime();
    let mut output = Vec::new();
    let reason = FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(b"/skill unknown\n/quit\n"), &mut output)
        .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("Usage: /skill list | /skill add | /skill show"));
    assert_eq!(reason, ShutdownReason::UserQuit);
    runtime.finish_and_join(reason);
}

#[test]
fn add_opens_a_guided_typed_review_and_never_commits_before_confirmation() {
    let runtime = support::runtime();
    let client = runtime.client();
    let input = Cursor::new(
        b"/skill add\n:name Fallback Research\n:description Optional command workflow.\n:use-when Use during research.\n:tag add fallback\n:instructions Keep evidence exact.\n:review\nq\n/quit\n",
    );
    let mut output = Vec::new();
    let reason = FallbackRunner::new(client.clone(), false)
        .run(input, &mut output)
        .unwrap();
    let text = String::from_utf8(output).unwrap();

    assert!(text.contains("Create skill editor"));
    assert!(text.contains("Skill creation review"));
    assert!(text.contains("version 1"));
    assert!(text.contains("Confirmation did not match; skill review retained."));
    assert_eq!(reason, ShutdownReason::UserQuit);
    assert!(matches!(
        client.submit(ApplicationCommand::ShowSkill {
            selector: SkillSelector::from_input("Fallback Research").unwrap(),
        }),
        Err(RuntimeError::Application(AppError::SkillNotFound))
    ));
    runtime.finish_and_join(reason);
}

#[test]
fn add_confirmation_uses_the_registered_review_and_creates_once() {
    let runtime = support::runtime();
    let client = runtime.client();
    let input = Cursor::new(
        b"/skill add\n:name Fallback Research\n:description Optional command workflow.\n:use-when Use during research.\n:tag add fallback\n:instructions Keep evidence exact.\n:review\ncreate\n/skill show \"Fallback Research\"\n/quit\n",
    );
    let mut output = Vec::new();
    let reason = FallbackRunner::new(client, false)
        .run(input, &mut output)
        .unwrap();
    let text = String::from_utf8(output).unwrap();

    assert!(text.contains("Skill creation review"));
    assert!(text.contains("Skill created:"));
    assert!(text.contains("Display name: Fallback Research"));
    assert_eq!(reason, ShutdownReason::UserQuit);
    runtime.finish_and_join(reason);
}

#[test]
fn assignment_stages_the_exact_active_ref_and_bare_q_never_commits_or_quits() {
    let runtime = support::runtime();
    let client = runtime.client();
    create_profile(&client, "Fallback Agent");

    let mut output = Vec::new();
    let reason = FallbackRunner::new(client.clone(), false)
        .run(
            Cursor::new(b"/skill assign \"Evidence Review\" \"Fallback Agent\"\nq\n/quit\n"),
            &mut output,
        )
        .unwrap();
    let text = String::from_utf8(output).unwrap();

    assert!(text.contains("Skill assignment review: assign"));
    assert!(text.contains("exact version 1"));
    assert!(text.contains("Confirmation did not match; skill review retained."));
    assert_eq!(reason, ShutdownReason::UserQuit);

    let outcome = client
        .submit(ApplicationCommand::ShowAgentProfile {
            selector: ai_stock_forum::app::AgentProfileSelector::from_input("Fallback Agent")
                .unwrap(),
        })
        .unwrap();
    let CommandView::AgentProfile(view) = outcome.view else {
        panic!("expected profile view")
    };
    assert!(view.profile.skill_refs().is_empty());
    runtime.finish_and_join(reason);
}

#[test]
fn assign_and_unassign_commit_only_through_their_registered_reviews() {
    let runtime = support::runtime();
    let client = runtime.client();
    create_profile(&client, "Fallback Agent");

    let mut output = Vec::new();
    let reason = FallbackRunner::new(client.clone(), false)
        .run(
            Cursor::new(
                b"/skill assign \"Evidence Review\" \"Fallback Agent\" 1\nassign\n/skill unassign \"Evidence Review\" \"Fallback Agent\"\nunassign\n/quit\n",
            ),
            &mut output,
        )
        .unwrap();
    let text = String::from_utf8(output).unwrap();

    assert!(text.contains("Skill assignment review: assign"));
    assert!(text.contains("Agent skill assigned:"));
    assert!(text.contains("Skill assignment review: unassign"));
    assert!(text.contains("Agent skill unassigned:"));
    assert_eq!(reason, ShutdownReason::UserQuit);

    let outcome = client
        .submit(ApplicationCommand::ShowAgentProfile {
            selector: ai_stock_forum::app::AgentProfileSelector::from_input("Fallback Agent")
                .unwrap(),
        })
        .unwrap();
    let CommandView::AgentProfile(view) = outcome.view else {
        panic!("expected profile view")
    };
    assert!(view.profile.skill_refs().is_empty());
    runtime.finish_and_join(reason);
}

#[test]
fn explicit_newer_version_routes_to_upgrade_without_changing_the_requested_ref() {
    let temporary_directory = TempDir::new().unwrap();
    let paths = AppPaths::for_test(temporary_directory.path());
    let mut service = ApplicationService::bootstrap(
        &paths,
        Arc::new(support::TestClock::new()),
        Arc::new(support::TestIds::new()),
    )
    .unwrap();

    let first = skill("Versioned Fallback", "Version one.");
    let create_preview = service.preview_skill_creation(first.clone()).unwrap();
    service
        .execute_user(ApplicationCommand::CreateSkill {
            skill_id: create_preview.skill_id,
            candidate: first,
            review_token: create_preview.review_token,
            review_digest: create_preview.review_digest,
        })
        .unwrap();
    let first_ref = match service
        .execute_user(ApplicationCommand::ShowSkillVersion {
            selector: SkillSelector::from(create_preview.skill_id),
            version: ObjectVersion::new(1).unwrap(),
        })
        .unwrap()
        .view
    {
        CommandView::SkillVersion(view) => view.skill_ref,
        _ => panic!("expected version one"),
    };

    let created_profile = service
        .execute_user(ApplicationCommand::CreateAgentProfile {
            draft: profile("Upgrade Agent"),
            template_provenance: None,
        })
        .unwrap();
    let CommandView::AgentProfileCreated(created_profile) = created_profile.view else {
        panic!("expected created profile")
    };
    let assign_preview = service
        .preview_agent_skill_assignment(
            created_profile.profile_id,
            created_profile.profile_version_id,
            first_ref.clone(),
        )
        .unwrap();
    let assigned = service
        .execute_user(ApplicationCommand::AssignAgentSkill {
            profile_id: assign_preview.profile_id,
            expected_active_profile_version_id: assign_preview.expected_active_profile_version_id,
            skill: first_ref.clone(),
            review_token: assign_preview.review_token,
            review_digest: assign_preview.review_digest,
        })
        .unwrap();
    let CommandView::AgentSkillAssigned(assigned) = assigned.view else {
        panic!("expected assigned profile")
    };

    let second = skill("Versioned Fallback", "Version two.");
    let version_preview = service
        .preview_skill_version(
            create_preview.skill_id,
            first_ref.skill_version_id(),
            second.clone(),
        )
        .unwrap();
    service
        .execute_user(ApplicationCommand::ActivateSkillVersion {
            skill_id: version_preview.skill_id,
            expected_active_version_id: first_ref.skill_version_id(),
            candidate: second,
            review_token: version_preview.review_token,
            review_digest: version_preview.review_digest,
        })
        .unwrap();

    let runtime = ApplicationRuntime::spawn_application(service, 32).unwrap();
    let client = runtime.client();
    let mut output = Vec::new();
    let reason = FallbackRunner::new(client.clone(), false)
        .run(
            Cursor::new(b"/skill assign \"Versioned Fallback\" \"Upgrade Agent\" 2\nupgrade\n/quit\n"),
            &mut output,
        )
        .unwrap();
    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("Skill assignment review: upgrade"));
    assert!(text.contains("exact version 2"));
    assert!(text.contains("Agent skill upgraded:"));

    let outcome = client
        .submit(ApplicationCommand::ShowAgentProfile {
            selector: ai_stock_forum::app::AgentProfileSelector::from_input("Upgrade Agent")
                .unwrap(),
        })
        .unwrap();
    let CommandView::AgentProfile(view) = outcome.view else {
        panic!("expected profile view")
    };
    assert_eq!(view.profile.skill_refs().len(), 1);
    assert_eq!(view.profile.skill_refs()[0].version().get(), 2);
    assert_ne!(view.profile.skill_refs()[0], first_ref);
    assert_eq!(view.profile.supersedes(), Some(assigned.profile_version_id));
    runtime.finish_and_join(reason).unwrap();
}
