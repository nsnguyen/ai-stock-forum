mod support;

use std::sync::Arc;

use ai_stock_forum::{
    app::{
        AppError, ApplicationCommand, AuthorizationDecision, CommandEnvelope, CommandView,
        InputRejection, InputRejectionCategory, SafeToken, ShutdownDisposition, ShutdownReason,
    },
    domain::{Actor, CommandId, CorrelationId},
    policy::{Capability, PolicyDecision},
};
use uuid::Uuid;

fn envelope(id: u128, correlation: u128, command: ApplicationCommand) -> CommandEnvelope {
    CommandEnvelope {
        command_id: CommandId::from_uuid(Uuid::from_u128(id)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(correlation)),
        actor: Actor::Human,
        command,
    }
}

fn commands() -> Vec<(ApplicationCommand, Capability, &'static str)> {
    vec![
        (ApplicationCommand::ShowHelp, Capability::HelpRead, "help_viewed"),
        (ApplicationCommand::ShowStatus, Capability::StatusRead, "status_viewed"),
        (
            ApplicationCommand::ShowSetupStatus,
            Capability::SetupStatusRead,
            "setup_status_viewed",
        ),
        (
            ApplicationCommand::audit_tail(7).unwrap(),
            Capability::AuditRead,
            "audit_tail_viewed",
        ),
        (
            ApplicationCommand::RejectInput(InputRejection::from_input(
                InputRejectionCategory::Unknown,
                Some(SafeToken::new("/unknown").unwrap()),
                b"unretained input",
            )),
            Capability::HelpRead,
            "command_rejected",
        ),
        (
            ApplicationCommand::RequestShutdown,
            Capability::Shutdown,
            "shutdown_requested",
        ),
    ]
}

#[test]
fn status_flows_through_event_and_projection_transaction() {
    let mut app = support::app();
    let outcome = app.execute_user(ApplicationCommand::ShowStatus).unwrap();
    let CommandView::Status(view) = outcome.view else {
        panic!("status view")
    };
    assert_eq!(view.installation_id, app.installation_id());
    assert_eq!(view.session_id, app.session_id());
    assert_eq!(outcome.committed_events.len(), 1);
    assert_eq!(outcome.committed_events[0].correlation_id, outcome.correlation_id);
    assert_eq!(app.persisted_last_sequence(), outcome.committed_events[0].sequence);
}

#[test]
fn setup_status_does_not_invent_configuration() {
    let mut app = support::app();
    let outcome = app
        .execute_user(ApplicationCommand::ShowSetupStatus)
        .unwrap();
    assert!(matches!(
        outcome.view,
        CommandView::SetupStatus(ref view) if view.is_not_started()
    ));
    assert_eq!(app.count_rows("setup_drafts"), 0);
    assert_eq!(app.count_rows("installation_configuration_versions"), 0);
    assert_eq!(app.count_rows("capability_readiness"), 0);
    assert_eq!(app.count_rows("approval_records"), 0);
}

#[test]
fn audit_tail_includes_its_own_committed_event() {
    let mut app = support::app();
    app.execute_user(ApplicationCommand::ShowHelp).unwrap();
    let outcome = app
        .execute_user(ApplicationCommand::audit_tail(20).unwrap())
        .unwrap();
    let CommandView::AuditTail(view) = outcome.view else {
        panic!("audit view")
    };
    assert_eq!(view.entries.last().unwrap().kind, "audit_tail_viewed");
}

#[test]
fn quit_requests_but_does_not_fake_session_completion() {
    let mut app = support::app();
    let outcome = app
        .execute_user(ApplicationCommand::RequestShutdown)
        .unwrap();
    assert_eq!(outcome.shutdown, ShutdownDisposition::Requested);
    assert_eq!(app.event_count("process_session_ended"), 0);
}

#[test]
fn every_command_uses_its_exact_capability_event_and_typed_view() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let clock_before = app.clock.calls();
    let ids_before = app.ids.calls();

    for (offset, (command, _, event_kind)) in commands().into_iter().enumerate() {
        let outcome = app
            .execute(envelope(100 + offset as u128, 200 + offset as u128, command))
            .unwrap();
        assert_eq!(outcome.committed_events[0].event.kind(), event_kind);
        assert!(matches!(
            (&outcome.view, event_kind),
            (CommandView::Help(_), "help_viewed")
                | (CommandView::Status(_), "status_viewed")
                | (CommandView::SetupStatus(_), "setup_status_viewed")
                | (CommandView::AuditTail(_), "audit_tail_viewed")
                | (CommandView::InputRejected(_), "command_rejected")
                | (CommandView::Shutdown(_), "shutdown_requested")
        ));
    }

    assert_eq!(
        policy.capabilities(),
        commands()
            .into_iter()
            .map(|(_, capability, _)| capability)
            .collect::<Vec<_>>()
    );
    assert_eq!(policy.calls(), 6);
    assert_eq!(app.clock.calls() - clock_before, 6);
    assert_eq!(app.ids.calls() - ids_before, 6);
}

#[test]
fn denied_commands_are_typed_inert_and_consume_no_clock_or_ids() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Denied(
        PolicyDecision::Denied,
    ));
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let events_before = app.count_rows("event_stream");
    let sequence_before = app.persisted_last_sequence();
    let clock_before = app.clock.calls();
    let ids_before = app.ids.calls();

    for (offset, (command, capability, _)) in commands().into_iter().enumerate() {
        assert_eq!(
            app.execute(envelope(300 + offset as u128, 400 + offset as u128, command))
                .unwrap_err(),
            AppError::CapabilityDenied {
                capability,
                decision: PolicyDecision::Denied,
            }
        );
    }

    assert_eq!(policy.calls(), 6);
    assert_eq!(app.clock.calls(), clock_before);
    assert_eq!(app.ids.calls(), ids_before);
    assert_eq!(app.count_rows("event_stream"), events_before);
    assert_eq!(app.persisted_last_sequence(), sequence_before);
}

#[test]
fn approval_required_commands_are_typed_inert_and_consume_no_clock_or_ids() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::ApprovalRequired);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let events_before = app.count_rows("event_stream");
    let sequence_before = app.persisted_last_sequence();
    let clock_before = app.clock.calls();
    let ids_before = app.ids.calls();

    for (offset, (command, capability, _)) in commands().into_iter().enumerate() {
        assert_eq!(
            app.execute(envelope(500 + offset as u128, 600 + offset as u128, command))
                .unwrap_err(),
            AppError::ApprovalRequired { capability }
        );
    }

    assert_eq!(policy.calls(), 6);
    assert_eq!(app.clock.calls(), clock_before);
    assert_eq!(app.ids.calls(), ids_before);
    assert_eq!(app.count_rows("event_stream"), events_before);
    assert_eq!(app.persisted_last_sequence(), sequence_before);
}

#[test]
fn command_retry_returns_the_exact_outcome_without_duplicate_events_or_dependencies() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let command = envelope(700, 701, ApplicationCommand::audit_tail(20).unwrap());
    let first = app.execute(command.clone()).unwrap();
    app.execute_user(ApplicationCommand::ShowHelp).unwrap();
    let event_count = app.count_rows("event_stream");
    let clock_before = app.clock.calls();
    let ids_before = app.ids.calls();
    let policy_before = policy.calls();

    let replay = app.execute(command).unwrap();

    assert_eq!(replay, first);
    assert_eq!(app.count_rows("event_stream"), event_count);
    assert_eq!(app.clock.calls(), clock_before);
    assert_eq!(app.ids.calls(), ids_before);
    assert_eq!(policy.calls(), policy_before);
}

#[test]
fn same_command_id_with_different_request_conflicts_deterministically() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    app.execute(envelope(800, 801, ApplicationCommand::ShowHelp))
        .unwrap();
    let event_count = app.count_rows("event_stream");
    let clock_before = app.clock.calls();
    let ids_before = app.ids.calls();
    let policy_before = policy.calls();

    assert_eq!(
        app.execute(envelope(800, 801, ApplicationCommand::ShowStatus))
            .unwrap_err(),
        AppError::CommandConflict
    );
    assert_eq!(
        app.execute(CommandEnvelope {
            correlation_id: CorrelationId::from_uuid(Uuid::from_u128(999)),
            ..envelope(800, 801, ApplicationCommand::ShowHelp)
        })
        .unwrap_err(),
        AppError::CommandConflict
    );
    assert_eq!(app.count_rows("event_stream"), event_count);
    assert_eq!(app.clock.calls(), clock_before);
    assert_eq!(app.ids.calls(), ids_before);
    assert_eq!(policy.calls(), policy_before);
}

#[test]
fn execution_reloads_authoritative_projection_instead_of_using_stale_boot_state() {
    let mut app = support::app();
    let before = app.persisted_last_sequence();
    app.append_external_help_event();

    let outcome = app.execute_user(ApplicationCommand::ShowStatus).unwrap();

    assert_eq!(outcome.committed_events[0].sequence, before + 2);
    assert_eq!(app.persisted_last_sequence(), before + 2);
}

#[test]
fn projection_failure_rolls_back_event_projection_and_command_receipt() {
    let mut app = support::app();
    app.install_projection_failure();
    let events_before = app.count_rows("event_stream");
    let sequence_before = app.persisted_last_sequence();

    assert!(matches!(
        app.execute(envelope(900, 901, ApplicationCommand::ShowStatus)),
        Err(AppError::Persistence(_))
    ));
    assert_eq!(app.count_rows("event_stream"), events_before);
    assert_eq!(app.persisted_last_sequence(), sequence_before);
}

#[test]
fn rejected_input_retains_only_bounded_redacted_metadata() {
    let mut app = support::app();
    let secret = b"broker-token=super-secret-value";
    let outcome = app
        .execute_user(ApplicationCommand::RejectInput(InputRejection::from_input(
            InputRejectionCategory::Malformed,
            None,
            secret,
        )))
        .unwrap();

    assert!(!format!("{outcome:?}").contains("super-secret-value"));
    assert!(!app.last_payload_json().contains("super-secret-value"));
}

#[test]
fn finish_records_the_real_terminal_reason_once() {
    let mut app = support::app();
    let clock_before = app.clock.calls();
    let ids_before = app.ids.calls();

    app.finish(ShutdownReason::UserQuit).unwrap();
    app.finish(ShutdownReason::ApplicationError).unwrap();

    assert_eq!(app.event_count("process_session_ended"), 1);
    assert_eq!(app.clock.calls() - clock_before, 1);
    assert_eq!(app.ids.calls() - ids_before, 2);
}
