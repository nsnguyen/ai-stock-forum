use ai_stock_forum::ui::command::FallbackRunner;
use std::{collections::BTreeSet, io::Cursor};

mod support;

#[test]
fn fresh_run_restart_and_audit_replay_satisfy_phase_zero() {
    let fixture = support::persistent_fixture();

    let first_runtime = fixture.runtime();
    let mut first_output = Vec::new();
    let first_reason = FallbackRunner::new(first_runtime.client(), false)
        .run(
            Cursor::new(b"/help\n/status\n/setup status\n/not-a-command\n/quit\n"),
            &mut first_output,
        )
        .unwrap();
    first_runtime.finish_and_join(first_reason).unwrap();

    assert_eq!(
        String::from_utf8(first_output).unwrap(),
        concat!(
            "Available commands:\n",
            "  /help\n",
            "  /status\n",
            "  /setup status\n",
            "  /audit tail [limit: 1-100]\n",
            "  /quit\n",
            "Installation: ready\n",
            "Session: active\n",
            "Setup: not started\n",
            "Guided setup is not implemented in Phase 0.\n",
            "Input rejected: unknown command /not-a-command.\n",
            "Shutting down.\n",
        )
    );

    let installation = fixture.installation_id();
    let first_run_events = fixture.events();
    assert_eq!(
        first_run_events
            .iter()
            .map(|event| event.kind.as_str())
            .collect::<Vec<_>>(),
        vec![
            "installation_initialized",
            "process_session_started",
            "help_viewed",
            "status_viewed",
            "setup_status_viewed",
            "command_rejected",
            "shutdown_requested",
            "process_session_ended",
        ]
    );
    let first_run_event_ids = first_run_events
        .iter()
        .map(|event| event.event_id)
        .collect::<Vec<_>>();
    assert_eq!(
        first_run_event_ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .len(),
        first_run_event_ids.len()
    );
    let session_id = payload_string(&first_run_events[1], "session_id");
    let first_run_summaries = vec![
        format!("installation initialized: {installation}"),
        format!("process session started: {session_id}"),
        "help viewed".to_owned(),
        "status viewed".to_owned(),
        "setup status viewed".to_owned(),
        "command rejected: category=unknown, token=/not-a-command, bytes=14".to_owned(),
        "shutdown requested".to_owned(),
        format!("process session ended: {session_id}, reason=UserQuit"),
    ];

    for table in [
        "setup_drafts",
        "installation_configuration_versions",
        "active_installation_configuration",
        "setup_step_outcomes",
        "capability_readiness",
        "approval_records",
    ] {
        assert_eq!(fixture.count_rows(table), 0, "unexpected row in {table}");
    }
    for forbidden_table in ["credentials", "broker_accounts"] {
        assert!(!fixture.table_exists(forbidden_table));
    }

    fixture.remove_recoverable_projection_state();
    assert_eq!(fixture.count_rows("installation_projection"), 0);
    assert_eq!(fixture.count_rows("process_session_projection"), 0);
    assert_eq!(fixture.count_rows("projection_metadata"), 0);

    let second_runtime = fixture.runtime();
    let mut second_output = Vec::new();
    let second_reason = FallbackRunner::new(second_runtime.client(), false)
        .run(Cursor::new(b"/audit tail 100\n/quit\n"), &mut second_output)
        .unwrap();
    second_runtime.finish_and_join(second_reason).unwrap();

    assert_eq!(fixture.installation_id(), installation);
    assert_eq!(fixture.event_count("projection_rebuilt"), 1);
    let restarted_events = fixture.events();
    assert_eq!(
        &restarted_events[..first_run_events.len()],
        first_run_events.as_slice()
    );
    assert_eq!(
        restarted_events[first_run_events.len()].kind,
        "projection_rebuilt"
    );
    assert_eq!(
        restarted_events[first_run_events.len()].payload_json,
        format!(r#"{{"through_sequence":{}}}"#, first_run_events.len())
    );

    let audit_output = String::from_utf8(second_output).unwrap();
    assert!(audit_output.starts_with("Audit tail (limit 100):\n"));
    assert_audit_entries_in_order(&audit_output, &first_run_events, &first_run_summaries);
    assert!(audit_output.contains("projection_rebuilt"));
    assert!(audit_output.contains(&format!(
        "projection rebuilt through sequence {}",
        first_run_events.len()
    )));
    assert!(audit_output.ends_with("Shutting down.\n"));

    fixture.verify_event_stream().unwrap();
    fixture.assert_projection_rows_match_event_stream();
}

fn payload_string(event: &support::PersistedEvent, field: &str) -> String {
    serde_json::from_str::<serde_json::Value>(&event.payload_json).unwrap()[field]
        .as_str()
        .unwrap()
        .to_owned()
}

fn assert_audit_entries_in_order(
    output: &str,
    events: &[support::PersistedEvent],
    summaries: &[String],
) {
    let mut lines = output.lines();
    assert_eq!(lines.next(), Some("Audit tail (limit 100):"));
    for (event, summary) in events.iter().zip(summaries) {
        let sequence_prefix = format!("  #{} ", event.sequence);
        let line = lines
            .find(|line| line.starts_with(&sequence_prefix))
            .unwrap_or_else(|| panic!("missing audit sequence {}", event.sequence));
        assert!(line.contains(&format!(" {} ", event.kind)));
        assert!(line.ends_with(summary));
    }
}
