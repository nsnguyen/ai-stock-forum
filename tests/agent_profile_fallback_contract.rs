mod support;

use std::io::Cursor;

use ai_stock_forum::{
    agents::{AgentProfileDraft, AgentRole, builtin_profile_templates},
    app::{ApplicationCommand, CommandView, ShutdownReason},
    domain::AgentProfileId,
    ui::command::{FallbackParsedLine, FallbackRunner, parse_fallback_line},
};

fn run_script(
    client: ai_stock_forum::runtime::RuntimeClient,
    script: impl AsRef<[u8]>,
) -> (ShutdownReason, String) {
    let mut output = Vec::new();
    let reason = FallbackRunner::new(client, false)
        .run(Cursor::new(script.as_ref()), &mut output)
        .unwrap();
    (reason, String::from_utf8(output).unwrap())
}

fn create_profile(
    client: &ai_stock_forum::runtime::RuntimeClient,
    name: &str,
) -> ai_stock_forum::app::AgentProfileCreatedView {
    let template = &builtin_profile_templates()[0];
    let mut draft = template.copy_to_draft().unwrap();
    draft.display_name = name.to_owned();
    let outcome = client
        .submit(ApplicationCommand::CreateAgentProfile {
            draft,
            template_provenance: Some(template.provenance()),
        })
        .unwrap();
    let CommandView::AgentProfileCreated(created) = outcome.view else {
        panic!("expected created profile view");
    };
    created
}

fn profile_count(client: &ai_stock_forum::runtime::RuntimeClient) -> usize {
    let outcome = client.submit(ApplicationCommand::ListAgentProfiles).unwrap();
    let CommandView::AgentProfiles(view) = outcome.view else {
        panic!("expected agent profile list");
    };
    view.profiles.len()
}

fn profile_history_len(
    client: &ai_stock_forum::runtime::RuntimeClient,
    profile_id: AgentProfileId,
) -> usize {
    let outcome = client
        .submit(ApplicationCommand::ShowAgentProfileHistory { profile_id })
        .unwrap();
    let CommandView::AgentProfileHistory(view) = outcome.view else {
        panic!("expected agent profile history");
    };
    view.versions.len()
}

fn edited_script(profile_id: AgentProfileId, confirmation: &str) -> String {
    format!(
        "agent edit {profile_id}\n:role custom\n:next\nFallback Editor\n:next\nEdited in the fallback workflow.\n:next\nquality research\n:tag remove growth\n:tag remove catalysts\n:tag add quality\n:next\nCalm and exact.\n:next\nCite primary evidence.\n:next\n:provider local\n:model analyst-v2\n:next\n:review\n:activate\n{confirmation}\n"
    )
}

#[test]
fn parser_accepts_only_the_six_exact_agent_forms_and_typed_ids() {
    let id = "00000000-0000-0000-0000-00000000002a";
    assert!(matches!(
        parse_fallback_line(b"agent list"),
        FallbackParsedLine::Command(ApplicationCommand::ListAgentProfiles)
    ));
    assert!(matches!(
        parse_fallback_line(format!("agent show {id}").as_bytes()),
        FallbackParsedLine::Command(ApplicationCommand::ShowAgentProfile { .. })
    ));
    assert!(matches!(
        parse_fallback_line(format!("agent history {id}").as_bytes()),
        FallbackParsedLine::Command(ApplicationCommand::ShowAgentProfileHistory { .. })
    ));

    for accepted in [
        "agent create",
        "agent create builtin.bull",
        &format!("agent edit {id}"),
    ] {
        assert!(
            !matches!(
                parse_fallback_line(accepted.as_bytes()),
                FallbackParsedLine::Command(ApplicationCommand::RejectInput(_))
            ),
            "expected accepted agent form: {accepted}"
        );
    }

    for rejected in [
        "agent",
        "agent list extra",
        "agent show",
        "agent show not-a-uuid",
        "agent history not-a-uuid",
        "agent create unknown-template",
        "agent edit not-a-uuid",
        "agent edit 00000000-0000-0000-0000-00000000002a extra",
        "agent delete 00000000-0000-0000-0000-00000000002a",
    ] {
        assert!(
            matches!(
                parse_fallback_line(rejected.as_bytes()),
                FallbackParsedLine::Command(ApplicationCommand::RejectInput(_))
            ),
            "expected rejected agent form: {rejected}"
        );
    }
}

#[test]
fn create_selects_a_pinned_template_edits_fields_and_waits_for_yes() {
    let fixture = support::runtime();
    let client = fixture.client();

    let (_, selection) = run_script(client.clone(), "agent create\n:cancel\n");
    assert!(selection.contains("Profile templates:"));
    assert!(selection.contains("builtin.bull"));
    assert!(selection.contains("Profile creation cancelled."));
    assert_eq!(profile_count(&client), 0);

    let script = concat!(
        "agent create builtin.bull\n",
        ":role custom\n",
        ":next\n",
        "Fallback Analyst\n",
        ":next\n",
        "Edited description.\n",
        ":next\n",
        "special situations\n",
        ":tag remove growth\n",
        ":tag remove catalysts\n",
        ":tag add catalysts\n",
        ":next\n",
        "Patient and skeptical.\n",
        ":next\n",
        "Separate facts from estimates.\n",
        ":next\n",
        ":provider local\n",
        ":model analyst-v1\n",
        ":next\n",
        ":review\n",
        ":activate\n",
        "yes\n",
    );
    let (_, output) = run_script(client.clone(), script);
    assert!(output.contains("Create profile review"));
    assert!(output.contains("Confirm activation? [y/yes or n/no]"));
    assert!(output.contains("Agent profile created:"));

    let listed = client.submit(ApplicationCommand::ListAgentProfiles).unwrap();
    let CommandView::AgentProfiles(listed) = listed.view else {
        panic!("expected list view");
    };
    assert_eq!(listed.profiles.len(), 1);
    assert_eq!(listed.profiles[0].display_name, "Fallback Analyst");
    assert_eq!(listed.profiles[0].role, AgentRole::Custom);
    fixture.finish_and_join(ShutdownReason::InputClosed);
}

#[test]
fn edit_loads_active_version_previews_ordered_diffs_and_no_returns_to_review() {
    let fixture = support::runtime();
    let client = fixture.client();
    let created = create_profile(&client, "Original Analyst");

    let (_, rejected) = run_script(client.clone(), edited_script(created.profile_id, "no"));
    assert!(rejected.contains("Original Analyst"));
    let names = [
        "display_name",
        "description",
        "role",
        "primary_specialty",
        "specialty_tags",
        "personality",
        "instructions",
        "bindings",
    ];
    let review = rejected
        .rsplit("Edit profile review")
        .next()
        .expect("edit review section is rendered");
    let positions = names
        .map(|name| review.find(name).expect("ordered diff field is rendered"));
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(rejected.contains("Activation declined; returned to review."));
    assert_eq!(profile_history_len(&client, created.profile_id), 1);

    let (_, accepted) = run_script(client.clone(), edited_script(created.profile_id, "y"));
    assert!(accepted.contains("Confirm activation? [y/yes or n/no]"));
    assert!(accepted.contains("Agent profile version activated:"));
    assert_eq!(profile_history_len(&client, created.profile_id), 2);
    fixture.finish_and_join(ShutdownReason::InputClosed);
}

#[test]
fn eof_cancel_and_editor_prose_create_no_durable_draft_or_generic_log_entry() {
    for script in [
        "agent create builtin.bear\n:next\nprivate draft prose\n",
        "agent create builtin.bear\n:next\nprivate draft prose\n:cancel\n",
    ] {
        let fixture = support::runtime();
        let client = fixture.client();
        let (_, output) = run_script(client.clone(), script);
        assert_eq!(profile_count(&client), 0);

        let (_, audit) = run_script(client.clone(), "/audit tail 100\n");
        assert!(!audit.contains("private draft prose"));
        assert!(!audit.contains("unknown command private"));
        assert!(output.contains("private draft prose") || script.ends_with(":cancel\n"));
        fixture.finish_and_join(ShutdownReason::InputClosed);
    }
}

#[test]
fn list_show_and_history_are_deterministic_terminal_safe_typed_views() {
    let fixture = support::runtime();
    let client = fixture.client();
    let created = create_profile(&client, "Unsafe\\nName");
    let script = format!(
        "agent list\nagent show {}\nagent history {}\n",
        created.profile_id, created.profile_id
    );

    let (_, first) = run_script(client.clone(), &script);
    let (_, second) = run_script(client.clone(), &script);
    assert_eq!(first, second);
    assert!(first.contains("NAME | ROLE | SPECIALTY | READY | VERSION | ID"));
    assert!(first.contains("Display name: Unsafe\\\\nName"));
    for label in [
        "Description:",
        "Role:",
        "Primary specialty:",
        "Specialty tags:",
        "Personality:",
        "Instructions:",
        "Template provenance:",
        "Bindings readiness:",
        "Memory namespace ID:",
        "Policy reference:",
        "Created time:",
        "Digest:",
        "VERSION | VERSION ID | PREDECESSOR | CREATED | READY | DIGEST",
    ] {
        assert!(first.contains(label), "missing typed field {label}");
    }
    assert!(!first.contains("{\""));
    fixture.finish_and_join(ShutdownReason::InputClosed);
}

#[allow(dead_code)]
fn _draft_type_is_part_of_the_contract(_: AgentProfileDraft) {}
