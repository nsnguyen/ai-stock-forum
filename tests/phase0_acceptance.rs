use ai_stock_forum::ui::command::FallbackRunner;
use std::io::Cursor;

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

    let installation = fixture.installation_id();
    let event_count = fixture.event_count_all();
    assert_eq!(fixture.count_rows("setup_drafts"), 0);
    assert_eq!(fixture.count_rows("installation_configuration_versions"), 0);

    let second_runtime = fixture.runtime();
    let mut second_output = Vec::new();
    let second_reason = FallbackRunner::new(second_runtime.client(), false)
        .run(
            Cursor::new(b"/status\n/audit tail 100\n/quit\n"),
            &mut second_output,
        )
        .unwrap();
    second_runtime.finish_and_join(second_reason).unwrap();

    assert_eq!(fixture.installation_id(), installation);
    assert!(fixture.event_count_all() > event_count);
    let installation_text = installation.to_string();
    assert!(
        String::from_utf8(second_output)
            .unwrap()
            .contains(installation_text.as_str())
    );
    fixture.verify_event_stream().unwrap();
    fixture.assert_projection_matches_replay();
}
