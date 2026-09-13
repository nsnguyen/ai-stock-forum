use ai_stock_forum::{
    agents::AgentProfileVersionRef,
    app::{AgentProfileSelector, ApplicationCommand, MemoryEditPreview},
    domain::{
        Actor, AgentProfileId, AgentProfileVersionId, EventId, MemoryEntryId, MemoryEntryVersionId,
        MemoryNamespaceId, MemoryReviewToken, ObjectVersion, sha256,
    },
    memory::{
        ExpectedMemoryEntryState, MEMORY_PLAINTEXT_WARNING, MemoryEditReview, MemoryEntryDraft,
        MemoryEntryVersion, MemoryMutationKind, MemoryNoChange, MemoryPlaintextAcknowledgement,
    },
    ui::memory_editor::{
        MEMORY_PLAINTEXT_WARNING as EDITOR_WARNING, MemoryEditor, MemoryEditorEffect,
        MemoryEditorStep,
    },
};
use uuid::Uuid;

fn uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn selector() -> AgentProfileSelector {
    AgentProfileSelector::from(AgentProfileId::from_uuid(uuid(1)))
}

fn profile() -> AgentProfileVersionRef {
    AgentProfileVersionRef::new(
        AgentProfileId::from_uuid(uuid(1)),
        AgentProfileVersionId::from_uuid(uuid(2)),
        ObjectVersion::new(1).unwrap(),
        sha256(b"profile"),
    )
    .unwrap()
}

fn draft(key: &str, value: &str, tags: &[&str]) -> MemoryEntryDraft {
    MemoryEntryDraft::new(
        key.to_owned(),
        value.to_owned(),
        tags.iter().map(|tag| (*tag).to_owned()).collect(),
    )
    .unwrap()
}

fn present_entry() -> MemoryEntryVersion {
    MemoryEntryVersion::create_present(
        MemoryNamespaceId::from_uuid(uuid(3)),
        MemoryEntryId::from_uuid(uuid(4)),
        MemoryEntryVersionId::from_uuid(uuid(5)),
        draft(
            "Portfolio Thesis",
            "Own durable companies",
            &["research", "valuation"],
        ),
        Actor::Human,
        10,
        None,
        EventId::from_uuid(uuid(6)),
    )
    .unwrap()
}

fn present_entry_with_tags(tags: &[&str]) -> MemoryEntryVersion {
    MemoryEntryVersion::create_present(
        MemoryNamespaceId::from_uuid(uuid(12)),
        MemoryEntryId::from_uuid(uuid(13)),
        MemoryEntryVersionId::from_uuid(uuid(14)),
        draft("Escaped Tags", "Retain exact tags", tags),
        Actor::Human,
        12,
        None,
        EventId::from_uuid(uuid(15)),
    )
    .unwrap()
}

fn set_review(candidate: MemoryEntryDraft) -> MemoryEditPreview {
    MemoryEditPreview::Review(MemoryEditReview {
        profile: profile(),
        namespace_id: MemoryNamespaceId::from_uuid(uuid(3)),
        expected: ExpectedMemoryEntryState::Absent,
        operation: MemoryMutationKind::Set,
        candidate: Some(candidate),
        diff: Vec::new(),
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(uuid(7)),
        review_digest: sha256(b"review"),
    })
}

fn delete_review() -> MemoryEditPreview {
    MemoryEditPreview::Review(MemoryEditReview {
        profile: profile(),
        namespace_id: MemoryNamespaceId::from_uuid(uuid(3)),
        expected: ExpectedMemoryEntryState::Absent,
        operation: MemoryMutationKind::Delete,
        candidate: None,
        diff: Vec::new(),
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(uuid(8)),
        review_digest: sha256(b"delete review"),
    })
}

fn incomplete_set_review() -> MemoryEditPreview {
    MemoryEditPreview::Review(MemoryEditReview {
        profile: profile(),
        namespace_id: MemoryNamespaceId::from_uuid(uuid(3)),
        expected: ExpectedMemoryEntryState::Absent,
        operation: MemoryMutationKind::Set,
        candidate: None,
        diff: Vec::new(),
        plaintext_acknowledgement: MemoryPlaintextAcknowledgement::LocalPlaintextHistoryV1,
        review_token: MemoryReviewToken::from_uuid(uuid(11)),
        review_digest: sha256(b"incomplete set review"),
    })
}

fn editor_with_preview_request() -> (MemoryEditor, u64, MemoryEntryDraft) {
    let mut editor = MemoryEditor::for_create(selector());
    assert_eq!(
        editor.submit_line("thesis".to_owned()).unwrap(),
        MemoryEditorEffect::None
    );
    assert_eq!(
        editor.submit_line("older value".to_owned()).unwrap(),
        MemoryEditorEffect::None
    );
    let MemoryEditorEffect::Preview(request) = editor.submit_line("analysis".to_owned()).unwrap()
    else {
        panic!("purpose tags must request a preview");
    };
    (editor, request.generation, request.candidate)
}

#[test]
fn create_editor_starts_empty_and_reexports_plaintext_warning() {
    let editor = MemoryEditor::for_create(selector());

    assert_eq!(editor.step(), MemoryEditorStep::Key);
    assert_eq!(editor.generation(), 0);
    assert_eq!(editor.key_input(), "");
    assert_eq!(editor.value_input(), "");
    assert_eq!(editor.tags_input(), "");
    assert_eq!(editor.preview(), None);
    assert_eq!(editor.safe_error_code(), None);
    assert_eq!(EDITOR_WARNING, MEMORY_PLAINTEXT_WARNING);
}

#[test]
fn present_seed_starts_at_value_and_tombstone_seed_fails_safely() {
    let present = present_entry();
    let mut editor = MemoryEditor::for_set(selector(), present.clone()).unwrap();

    assert_eq!(editor.step(), MemoryEditorStep::Value);
    assert_eq!(editor.key_input(), "Portfolio Thesis");
    assert_eq!(editor.value_input(), "Own durable companies");
    assert_eq!(editor.tags_input(), "research, valuation");
    assert_eq!(editor.back(), MemoryEditorEffect::Cancelled);
    assert_eq!(editor.step(), MemoryEditorStep::Value);

    let tombstone = present
        .next_deleted(
            MemoryEntryVersionId::from_uuid(uuid(9)),
            Actor::Human,
            11,
            None,
            EventId::from_uuid(uuid(10)),
        )
        .unwrap();
    assert_eq!(
        MemoryEditor::for_set(selector(), tombstone)
            .unwrap_err()
            .code(),
        "invalid_memory_editor_seed"
    );
}

#[test]
fn direct_multiline_value_and_canonical_tags_advance_the_exact_steps() {
    let mut editor = MemoryEditor::for_create(selector());
    assert_eq!(
        editor.submit_line("  thesis  ".to_owned()).unwrap(),
        MemoryEditorEffect::None
    );
    assert_eq!(editor.step(), MemoryEditorStep::Value);
    assert_eq!(editor.key_input(), "thesis");

    assert_eq!(
        editor
            .submit_line("first line\nsecond line".to_owned())
            .unwrap(),
        MemoryEditorEffect::None
    );
    assert_eq!(editor.step(), MemoryEditorStep::PurposeTags);
    assert_eq!(editor.value_input(), "first line\nsecond line");

    let before_keyboard_rejection = editor.clone();
    assert_eq!(
        editor
            .submit_keyboard_line("research\nanalysis")
            .unwrap_err()
            .code(),
        "invalid_memory_editor_transition"
    );
    assert_eq!(editor, before_keyboard_rejection);

    let MemoryEditorEffect::Preview(request) = editor
        .submit_line(" research, analysis, , ".to_owned())
        .unwrap()
    else {
        panic!("canonical tags must request a preview");
    };
    assert_eq!(request.candidate.purpose_tags(), ["analysis", "research"]);
    assert_eq!(editor.tags_input(), "analysis, research");
}

#[test]
fn seeded_and_replaced_values_preserve_comma_and_backslash_tags_exactly() {
    let expected_tags = ["alpha", "beta, alpha", r"path\segment"];
    let mut editor = MemoryEditor::for_set(selector(), present_entry_with_tags(&expected_tags))
        .expect("valid present seed");

    assert_eq!(editor.draft().unwrap().purpose_tags(), expected_tags);
    assert_eq!(editor.tags_input(), r"alpha, beta\, alpha, path\\segment");

    editor.replace_value("Updated value".to_owned()).unwrap();
    assert_eq!(editor.draft().unwrap().purpose_tags(), expected_tags);
    assert_eq!(editor.tags_input(), r"alpha, beta\, alpha, path\\segment");
}

#[test]
fn create_tags_parse_escaped_delimiters_and_reject_malformed_escapes_without_mutation() {
    let mut editor = MemoryEditor::for_create(selector());
    editor.submit_line("thesis".to_owned()).unwrap();
    editor.submit_line("value".to_owned()).unwrap();

    let before_bad_escape = editor.clone();
    assert_eq!(
        editor
            .submit_line(format!("alpha{}", char::from_u32(92).unwrap()))
            .unwrap_err()
            .code(),
        "invalid_memory_editor_transition"
    );
    assert_eq!(editor, before_bad_escape);
    assert_eq!(
        editor
            .submit_line(r"alpha\x".to_owned())
            .unwrap_err()
            .code(),
        "invalid_memory_editor_transition"
    );
    assert_eq!(editor, before_bad_escape);

    let MemoryEditorEffect::Preview(request) = editor
        .submit_line(r"beta\, alpha, path\\segment".to_owned())
        .unwrap()
    else {
        panic!("escaped tags must request a preview");
    };
    assert_eq!(
        request.candidate.purpose_tags(),
        ["beta, alpha", r"path\segment"]
    );
    assert_eq!(editor.tags_input(), r"beta\, alpha, path\\segment");
}

#[test]
fn stale_preview_generation_cannot_replace_a_newer_draft() {
    let (mut editor, generation, _) = editor_with_preview_request();
    editor.replace_value("newer value".to_owned()).unwrap();
    let before_stale_response = editor.clone();

    assert!(!editor.apply_preview(
        generation,
        set_review(draft("thesis", "older value", &["analysis"]))
    ));
    assert_eq!(editor, before_stale_response);
    assert_eq!(editor.draft().unwrap().value(), "newer value");
    assert_eq!(editor.step(), MemoryEditorStep::PurposeTags);
}

#[test]
fn review_line_is_invalid_and_explicit_confirmation_uses_exact_review_binding() {
    let (mut editor, generation, candidate) = editor_with_preview_request();
    let review = set_review(candidate.clone());
    assert!(editor.apply_preview(generation, review.clone()));
    assert_eq!(editor.step(), MemoryEditorStep::Review);
    assert_eq!(
        editor.submit_line("confirm".to_owned()).unwrap_err().code(),
        "invalid_memory_editor_transition"
    );

    let MemoryEditorEffect::Confirm(command) = editor.confirm().unwrap() else {
        panic!("explicit confirmation must emit a command");
    };
    let MemoryEditPreview::Review(review) = review else {
        unreachable!();
    };
    assert_eq!(
        command,
        ApplicationCommand::SetMemoryEntry {
            profile: review.profile,
            expected: review.expected,
            candidate,
            review_token: review.review_token,
            review_digest: review.review_digest,
        }
    );
}

#[test]
fn no_change_has_no_confirmation_effect_and_delete_review_is_rejected() {
    let (mut editor, generation, _) = editor_with_preview_request();
    assert!(editor.apply_preview(
        generation,
        MemoryEditPreview::NoChange(MemoryNoChange::IdenticalContent)
    ));
    assert_eq!(editor.confirm().unwrap(), MemoryEditorEffect::None);

    editor.clear_review();
    let generation = editor.generation();
    assert!(editor.apply_preview(generation, delete_review()));
    assert_eq!(
        editor.confirm().unwrap_err().code(),
        "invalid_memory_editor_transition"
    );

    editor.clear_review();
    let generation = editor.generation();
    assert!(editor.apply_preview(generation, incomplete_set_review()));
    assert_eq!(
        editor.confirm().unwrap_err().code(),
        "invalid_memory_editor_transition"
    );
}

#[test]
fn errors_and_draft_replacements_invalidate_preview_without_recording_prose() {
    let (mut editor, generation, candidate) = editor_with_preview_request();
    assert!(editor.apply_preview(generation, set_review(candidate)));
    let review_generation = editor.generation();

    editor.report_error("preview_failed");
    assert_eq!(editor.safe_error_code(), Some("preview_failed"));
    assert_eq!(editor.preview(), None);
    assert_eq!(editor.generation(), review_generation + 1);

    editor.replace_value("replacement".to_owned()).unwrap();
    assert_eq!(editor.safe_error_code(), None);
    assert_eq!(editor.preview(), None);
    assert_eq!(editor.value_input(), "replacement");
    assert_eq!(editor.step(), MemoryEditorStep::PurposeTags);
}

#[test]
fn incomplete_preview_request_and_invalid_tag_submission_return_safe_errors() {
    let mut editor = MemoryEditor::for_create(selector());
    assert_eq!(
        editor.preview_request().unwrap_err().code(),
        "invalid_memory_editor_transition"
    );
    editor.submit_line("thesis".to_owned()).unwrap();
    editor.submit_line("value".to_owned()).unwrap();
    assert_eq!(
        editor
            .submit_line("analysis, ANALYSIS".to_owned())
            .unwrap_err()
            .code(),
        "invalid_memory_field"
    );
    assert_eq!(editor.step(), MemoryEditorStep::PurposeTags);
    assert_eq!(editor.tags_input(), "");
}
