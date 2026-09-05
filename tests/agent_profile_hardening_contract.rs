mod support;

use std::{io::Cursor, sync::Arc};

use ai_stock_forum::{
    agents::{AgentBindings, AgentProfileDraft, AgentRole},
    app::{
        AppError, ApplicationCommand, ApplicationService, CommandEnvelope, CommandView,
    },
    config::AppPaths,
    domain::{Actor, CommandId, CorrelationId, canonical_json_bytes, sha256},
    runtime::RuntimeError,
    ui::command::TextRenderer,
};
use rusqlite::Connection;
use tempfile::TempDir;
use uuid::Uuid;

fn envelope(id: u128, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(id + 100_000)),
        actor: Actor::Human,
        command,
    }
}

fn draft(name: &str) -> AgentProfileDraft {
    AgentProfileDraft::new(
        name.to_owned(),
        "Safe profile description.".to_owned(),
        AgentRole::Custom,
        "equity research".to_owned(),
        vec!["research".to_owned()],
        "Calm and skeptical personality.".to_owned(),
        "Cite primary evidence.".to_owned(),
        AgentBindings::default(),
        Vec::new(),
        Vec::new(),
    )
    .unwrap()
}

fn render_error(error: AppError) -> String {
    let mut bytes = Vec::new();
    TextRenderer::render_runtime_error(&RuntimeError::Application(error), &mut bytes).unwrap();
    String::from_utf8(bytes).unwrap()
}

fn assert_terminal_safe(text: &str) {
    assert_eq!(text.matches('\n').count(), 1);
    assert!(text.ends_with('\n'));
    assert!(!text[..text.len() - 1].chars().any(char::is_control));
    assert!(!text.contains('\u{2028}'));
    assert!(!text.contains('\u{2029}'));
    assert!(!text.contains('\u{202e}'));
}

#[test]
fn hostile_profile_text_is_rejected_before_ids_time_or_persistence_and_is_redacted() {
    let hostile_values = [
        ("esc", "credential=secret\u{1b}tail".to_owned()),
        ("osc", "credential=secret\u{1b}]0;title\u{7}".to_owned()),
        ("csi", "credential=secret\u{1b}[31mred".to_owned()),
        ("newline", "credential=secret\nforged".to_owned()),
        ("carriage_return", "credential=secret\rforged".to_owned()),
        ("tab", "credential=secret\tforged".to_owned()),
        ("nul", "credential=secret\0tail".to_owned()),
        ("line_separator", "credential=secret\u{2028}forged".to_owned()),
        (
            "paragraph_separator",
            "credential=secret\u{2029}forged".to_owned(),
        ),
        ("bidi_override", "credential=secret\u{202e}txt".to_owned()),
        (
            "over_limit_multibyte",
            format!("{}credential", "界".repeat(83)),
        ),
    ];

    for (offset, (label, hostile)) in hostile_values.into_iter().enumerate() {
        let mut app = support::app();
        let mut candidate = draft(&format!("Hostile Input Analyst {offset}"));
        candidate.description = hostile.clone();
        let before = (
            app.count_rows("event_stream"),
            app.count_rows("agent_profile_versions"),
            app.count_rows("active_agent_profiles"),
            app.count_rows("command_receipts"),
            app.count_rows("command_event_refs"),
            app.ids.calls(),
            app.clock.calls(),
        );

        let error = app
            .execute(envelope(
                50_000 + offset as u128,
                ApplicationCommand::CreateAgentProfile {
                    draft: candidate,
                    template_provenance: None,
                },
            ))
            .expect_err(label);

        assert!(matches!(error, AppError::Domain(_)), "case {label}");
        let displayed = error.to_string();
        assert!(!displayed.contains("credential"), "case {label}");
        assert!(!displayed.contains(&hostile), "case {label}");
        let rendered = render_error(error);
        assert_eq!(
            rendered,
            "Agent profile operation could not be completed.\n",
            "case {label}"
        );
        assert_terminal_safe(&rendered);
        assert_eq!(
            (
                app.count_rows("event_stream"),
                app.count_rows("agent_profile_versions"),
                app.count_rows("active_agent_profiles"),
                app.count_rows("command_receipts"),
                app.count_rows("command_event_refs"),
                app.ids.calls(),
                app.clock.calls(),
            ),
            before,
            "case {label}"
        );
    }
}

struct ReceiptFixture {
    _temporary_directory: TempDir,
    paths: AppPaths,
    app: ApplicationService,
}

impl ReceiptFixture {
    fn new() -> Self {
        let temporary_directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::for_test(temporary_directory.path());
        let app = ApplicationService::bootstrap(
            &paths,
            Arc::new(support::TestClock::new()),
            Arc::new(support::TestIds::new()),
        )
        .unwrap();
        Self {
            _temporary_directory: temporary_directory,
            paths,
            app,
        }
    }

    fn connection(&self) -> Connection {
        Connection::open(self.paths.database_path()).unwrap()
    }


    fn snapshot(&self) -> DurableState {
        Self::snapshot_path(&self.paths)
    }

    fn snapshot_path(paths: &AppPaths) -> DurableState {
        let connection = Connection::open(paths.database_path()).unwrap();
        let events = {
            let mut statement = connection
                .prepare(
                    "SELECT sequence, event_id, event_type, payload_json, event_digest
                     FROM event_stream ORDER BY sequence",
                )
                .unwrap();
            statement
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let versions = {
            let mut statement = connection
                .prepare(
                    "SELECT profile_id, profile_version_id, version, normalized_name,
                            content_digest, payload_json, source_event_sequence, created_at_ms
                     FROM agent_profile_versions ORDER BY source_event_sequence",
                )
                .unwrap();
            statement
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let active = {
            let mut statement = connection
                .prepare(
                    "SELECT profile_id, profile_version_id, version, normalized_name, readiness
                     FROM active_agent_profiles ORDER BY profile_id",
                )
                .unwrap();
            statement
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let receipts = {
            let mut statement = connection
                .prepare(
                    "SELECT command_id, command_fingerprint, request_json, capability,
                            policy_decision, outcome_json
                     FROM command_receipts ORDER BY command_id",
                )
                .unwrap();
            statement
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                })
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let refs = {
            let mut statement = connection
                .prepare(
                    "SELECT command_id, event_ordinal, event_id
                     FROM command_event_refs ORDER BY command_id, event_ordinal",
                )
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let projection = connection
            .query_row(
                "SELECT last_event_sequence, last_event_digest, projection_digest
                 FROM projection_metadata WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        DurableState {
            events,
            versions,
            active,
            receipts,
            refs,
            projection,
        }
    }

    fn into_parts(self) -> (TempDir, AppPaths, ApplicationService) {
        (self._temporary_directory, self.paths, self.app)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DurableState {
    events: Vec<(i64, String, String, String, String)>,
    versions: Vec<(String, String, i64, String, String, Vec<u8>, i64, i64)>,
    active: Vec<(String, String, i64, String, String)>,
    receipts: Vec<(String, String, String, String, String, String)>,
    refs: Vec<(String, i64, String)>,
    projection: (i64, Option<String>, String),
}

#[derive(Debug, Clone, Copy)]
enum StoredTamper {
    MalformedJson,
    NoncanonicalJson,
    InvalidUuid,
    UnknownRole,
    FingerprintDigest,
    EventDigest,
    MalformedEventPayload,
}

fn tamper(fixture: &ReceiptFixture, command_id: CommandId, kind: StoredTamper) {
    let connection = fixture.connection();
    if matches!(kind, StoredTamper::EventDigest | StoredTamper::MalformedEventPayload) {
        connection
            .execute_batch("DROP TRIGGER event_stream_no_update;")
            .unwrap();
        match kind {
            StoredTamper::EventDigest => {
                connection
                    .execute(
                        "UPDATE event_stream SET event_digest = ?1 WHERE event_id = (
                            SELECT event_id FROM command_event_refs WHERE command_id = ?2
                         )",
                        rusqlite::params!["a".repeat(64), command_id.to_string()],
                    )
                    .unwrap();
            }
            StoredTamper::MalformedEventPayload => {
                connection
                    .execute(
                        "UPDATE event_stream SET payload_json = ?1 WHERE event_id = (
                            SELECT event_id FROM command_event_refs WHERE command_id = ?2
                         )",
                        rusqlite::params![
                            "{\"credential\":\"malformed-event\u{2028}\"}",
                            command_id.to_string()
                        ],
                    )
                    .unwrap();
            }
            _ => unreachable!(),
        }
        return;
    }

    connection
        .execute_batch(
            "DROP TRIGGER command_receipts_no_update;
             PRAGMA ignore_check_constraints = ON;",
        )
        .unwrap();
    let original: String = connection
        .query_row(
            "SELECT request_json FROM command_receipts WHERE command_id = ?1",
            [command_id.to_string()],
            |row| row.get(0),
        )
        .unwrap();
    let (request_json, fingerprint) = match kind {
        StoredTamper::MalformedJson => {
            let request = "{\"credential\":\"malformed\"".to_owned();
            let digest = sha256(request.as_bytes()).to_string();
            (request, digest)
        }
        StoredTamper::NoncanonicalJson => {
            let value: serde_json::Value = serde_json::from_str(&original).unwrap();
            let request = serde_json::to_string_pretty(&value).unwrap();
            let digest = sha256(request.as_bytes()).to_string();
            (request, digest)
        }
        StoredTamper::InvalidUuid => {
            let mut value: serde_json::Value = serde_json::from_str(&original).unwrap();
            value["correlation_id"] =
                serde_json::json!("credential=invalid-uuid\u{1b}[31m");
            let request = String::from_utf8(canonical_json_bytes(&value).unwrap()).unwrap();
            let digest = sha256(request.as_bytes()).to_string();
            (request, digest)
        }
        StoredTamper::UnknownRole => {
            let mut value: serde_json::Value = serde_json::from_str(&original).unwrap();
            value["command"]["data"]["draft"]["role"] =
                serde_json::json!("credential=unknown-role\u{202e}");
            let request = String::from_utf8(canonical_json_bytes(&value).unwrap()).unwrap();
            let digest = sha256(request.as_bytes()).to_string();
            (request, digest)
        }
        StoredTamper::FingerprintDigest => (original, "b".repeat(64)),
        StoredTamper::EventDigest | StoredTamper::MalformedEventPayload => unreachable!(),
    };
    connection
        .execute(
            "UPDATE command_receipts
             SET request_json = ?1, command_fingerprint = ?2
             WHERE command_id = ?3",
            rusqlite::params![request_json, fingerprint, command_id.to_string()],
        )
        .unwrap();
}

#[test]
fn malformed_canonical_receipts_and_digest_tampering_fail_closed_without_leaks_or_writes() {
    for (offset, (kind, expected_code)) in [
        (StoredTamper::MalformedJson, "invalid_event_record"),
        (StoredTamper::NoncanonicalJson, "invalid_event_record"),
        (StoredTamper::InvalidUuid, "invalid_event_record"),
        (StoredTamper::UnknownRole, "invalid_event_record"),
        (StoredTamper::FingerprintDigest, "invalid_event_record"),
        (StoredTamper::EventDigest, "event_digest_mismatch"),
        (StoredTamper::MalformedEventPayload, "invalid_event_record"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut fixture = ReceiptFixture::new();
        let command = envelope(
            70_000 + offset as u128,
            ApplicationCommand::CreateAgentProfile {
                draft: draft(&format!("Tamper Analyst {offset}")),
                template_provenance: None,
            },
        );
        let created = fixture.app.execute(command.clone()).unwrap();
        assert!(matches!(created.view, CommandView::AgentProfileCreated(_)));
        tamper(&fixture, command.command_id, kind);
        let before = fixture.snapshot();

        let error = fixture.app.execute(command).unwrap_err();

        assert_eq!(error.code(), expected_code, "tamper {kind:?}");
        let displayed = error.to_string();
        assert!(!displayed.contains("credential"), "tamper {kind:?}");
        assert!(!displayed.contains('\u{1b}'), "tamper {kind:?}");
        assert!(!displayed.contains('\u{202e}'), "tamper {kind:?}");
        let rendered = render_error(error);
        assert_eq!(rendered, "Application command failed.\n");
        assert_terminal_safe(&rendered);
        assert_eq!(fixture.snapshot(), before, "tamper {kind:?}");
    }
}

#[test]
fn oversized_fallback_line_is_bounded_rejected_and_rendered_as_one_safe_line() {
    let fixture = ReceiptFixture::new();
    let before = fixture.snapshot();
    let (temporary_directory, paths, service) = fixture.into_parts();
    let runtime = ai_stock_forum::runtime::ApplicationRuntime::spawn_application(service, 1)
        .unwrap();
    let mut input = b"credential=oversized-secret\x1b[31m".to_vec();
    input.resize(32 * 1024, b'x');
    input.push(b'\n');
    let mut output = Vec::new();

    let reason = ai_stock_forum::ui::command::FallbackRunner::new(runtime.client(), false)
        .run(Cursor::new(input), &mut output)
        .unwrap();
    let rendered = String::from_utf8(output).unwrap();

    assert_eq!(reason, ai_stock_forum::app::ShutdownReason::InputClosed);
    assert_eq!(rendered, "Input rejected: input exceeds 4096 bytes.\n");
    assert_terminal_safe(&rendered);
    assert!(!rendered.contains("credential"));
    assert!(!rendered.contains("oversized-secret"));
    assert_eq!(ReceiptFixture::snapshot_path(&paths), before);

    runtime.finish_and_join(reason).unwrap();
    drop(temporary_directory);
}
