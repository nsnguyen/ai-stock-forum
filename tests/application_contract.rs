mod support;

use std::{
    sync::{Arc, Barrier},
    thread,
};

use ai_stock_forum::{
    app::{
        AppError, ApplicationCommand, AuthorizationDecision, CommandEnvelope, CommandView,
        InputRejection, InputRejectionCategory, SafeToken, ShutdownDisposition, ShutdownReason,
    },
    domain::{Actor, CommandId, CorrelationId},
    persistence::PersistenceError,
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
    assert_eq!(app.count_rows("command_receipts"), 6);
    assert_eq!(app.count_rows("command_event_refs"), 0);

    policy.set_decision(AuthorizationDecision::Granted);
    let policy_before = policy.calls();
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
    assert_eq!(policy.calls(), policy_before);
    assert_eq!(app.clock.calls(), clock_before);
    assert_eq!(app.ids.calls(), ids_before);
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
    assert_eq!(app.count_rows("command_receipts"), 6);
    assert_eq!(app.count_rows("command_event_refs"), 0);

    policy.set_decision(AuthorizationDecision::Granted);
    let policy_before = policy.calls();
    for (offset, (command, capability, _)) in commands().into_iter().enumerate() {
        assert_eq!(
            app.execute(envelope(500 + offset as u128, 600 + offset as u128, command))
                .unwrap_err(),
            AppError::ApprovalRequired { capability }
        );
    }
    assert_eq!(policy.calls(), policy_before);
    assert_eq!(app.clock.calls(), clock_before);
    assert_eq!(app.ids.calls(), ids_before);
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
    assert_eq!(app.count_rows("command_receipts"), 2);
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
    assert_eq!(app.count_rows("command_receipts"), 1);
}

#[test]
fn authoritative_events_ahead_of_projection_are_rejected_without_mutation_or_dependencies() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let projection_before = app.persisted_last_sequence();
    app.append_authoritative_help_event_without_projection();
    let event_sequence = app.max_event_sequence();
    let receipts_before = app.count_rows("command_receipts");
    let clock_before = app.clock.calls();
    let ids_before = app.ids.calls();
    let policy_before = policy.calls();

    assert!(matches!(
        app.execute(envelope(850, 851, ApplicationCommand::ShowStatus)),
        Err(AppError::Recovery(_))
    ));

    assert_eq!(app.persisted_last_sequence(), projection_before);
    assert_eq!(app.max_event_sequence(), event_sequence);
    assert_eq!(app.count_rows("command_receipts"), receipts_before);
    assert_eq!(app.clock.calls(), clock_before);
    assert_eq!(app.ids.calls(), ids_before);
    assert_eq!(policy.calls(), policy_before);
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
    assert_eq!(app.count_rows("command_receipts"), 0);
}

#[test]
fn receipt_and_outcome_failures_roll_back_every_command_effect() {
    for failure in [
        support::HookFailure::OutcomeMaterialization,
        support::HookFailure::ReceiptWrite,
    ] {
        let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
        let hook = Arc::new(support::TestCommandHook::failing(failure));
        let mut app = support::app_with_policy_and_hook(Arc::new(policy.clone()), hook);
        let events_before = app.count_rows("event_stream");
        let sequence_before = app.persisted_last_sequence();
        let clock_before = app.clock.calls();
        let ids_before = app.ids.calls();

        assert!(matches!(
            app.execute(envelope(1_000 + failure as u128, 1_100, ApplicationCommand::ShowStatus)),
            Err(AppError::Persistence(_))
        ));
        assert_eq!(app.count_rows("event_stream"), events_before);
        assert_eq!(app.persisted_last_sequence(), sequence_before);
        assert_eq!(app.count_rows("command_receipts"), 0);
        assert_eq!(app.count_rows("command_event_refs"), 0);
        assert_eq!(app.clock.calls() - clock_before, 1);
        assert_eq!(app.ids.calls() - ids_before, 1);
        assert_eq!(policy.calls(), 1);
    }
}

#[test]
fn durable_receipt_is_canonical_complete_and_orders_event_refs() {
    let mut app = support::app();
    let command = envelope(1_200, 1_201, ApplicationCommand::ShowStatus);
    let outcome = app.execute(command.clone()).unwrap();
    let (fingerprint, request_json, capability, decision, outcome_json) =
        app.receipt_row(command.command_id);

    assert_eq!(
        ai_stock_forum::domain::sha256(request_json.as_bytes()).as_str(),
        fingerprint
    );
    assert_eq!(capability, "status_read");
    assert_eq!(decision, "granted");
    for json in [&request_json, &outcome_json] {
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(
            ai_stock_forum::domain::canonical_json_bytes(&value).unwrap(),
            json.as_bytes()
        );
    }
    assert!(outcome_json.contains(&outcome.command_id.to_string()));
    assert_eq!(app.event_ref_ordinals(command.command_id), vec![0]);
}

#[test]
fn malformed_multi_event_order_is_rejected_before_dependencies() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let command = envelope(1_300, 1_301, ApplicationCommand::ShowHelp);
    app.execute(command.clone()).unwrap();
    app.shift_event_ref_to_ordinal_one(command.command_id);
    let clock_before = app.clock.calls();
    let ids_before = app.ids.calls();
    let policy_before = policy.calls();

    assert!(matches!(
        app.execute(command),
        Err(AppError::Persistence(_))
    ));
    assert_eq!(app.clock.calls(), clock_before);
    assert_eq!(app.ids.calls(), ids_before);
    assert_eq!(policy.calls(), policy_before);
}

#[test]
fn finished_service_rejects_commands_before_policy_ids_clock_or_writes() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    app.finish(ShutdownReason::UserQuit).unwrap();
    let receipts_before = app.count_rows("command_receipts");
    let events_before = app.count_rows("event_stream");
    let clock_before = app.clock.calls();
    let ids_before = app.ids.calls();
    let policy_before = policy.calls();

    assert_eq!(
        app.execute_user(ApplicationCommand::ShowStatus).unwrap_err(),
        AppError::LifecycleFinished
    );
    assert_eq!(app.count_rows("command_receipts"), receipts_before);
    assert_eq!(app.count_rows("event_stream"), events_before);
    assert_eq!(app.clock.calls(), clock_before);
    assert_eq!(app.ids.calls(), ids_before);
    assert_eq!(policy.calls(), policy_before);
}

#[test]
fn concurrent_same_command_uses_one_receipt_and_returns_one_exact_outcome() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let app = support::app_with_policy(Arc::new(policy.clone()));
    let mut first = app.peer();
    let mut second = app.peer();
    let command = envelope(1_400, 1_401, ApplicationCommand::ShowStatus);
    let barrier = Arc::new(Barrier::new(3));
    let first_barrier = barrier.clone();
    let first_command = command.clone();
    let first_handle = thread::spawn(move || {
        first_barrier.wait();
        first.execute(first_command)
    });
    let second_barrier = barrier.clone();
    let second_handle = thread::spawn(move || {
        second_barrier.wait();
        second.execute(command)
    });
    let events_before = app.count_rows("event_stream");
    let clock_before = app.clock.calls();
    let ids_before = app.ids.calls();
    barrier.wait();

    let first = first_handle.join().unwrap().unwrap();
    let second = second_handle.join().unwrap().unwrap();
    assert_eq!(first, second);
    assert_eq!(app.count_rows("command_receipts"), 1);
    assert_eq!(app.count_rows("command_event_refs"), 1);
    assert_eq!(app.count_rows("event_stream"), events_before + 1);
    assert_eq!(app.clock.calls() - clock_before, 1);
    assert_eq!(app.ids.calls() - ids_before, 1);
    assert_eq!(policy.calls(), 1);
}

#[test]
fn concurrent_conflicting_command_has_one_winner_and_one_deterministic_conflict() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let app = support::app_with_policy(Arc::new(policy.clone()));
    let mut first = app.peer();
    let mut second = app.peer();
    let barrier = Arc::new(Barrier::new(3));
    let first_barrier = barrier.clone();
    let first_handle = thread::spawn(move || {
        first_barrier.wait();
        first.execute(envelope(1_500, 1_501, ApplicationCommand::ShowHelp))
    });
    let second_barrier = barrier.clone();
    let second_handle = thread::spawn(move || {
        second_barrier.wait();
        second.execute(envelope(1_500, 1_502, ApplicationCommand::ShowStatus))
    });
    let events_before = app.count_rows("event_stream");
    let clock_before = app.clock.calls();
    let ids_before = app.ids.calls();
    barrier.wait();

    let results = [first_handle.join().unwrap(), second_handle.join().unwrap()];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(AppError::CommandConflict)))
            .count(),
        1
    );
    assert_eq!(app.count_rows("command_receipts"), 1);
    assert_eq!(app.count_rows("command_event_refs"), 1);
    assert_eq!(app.count_rows("event_stream"), events_before + 1);
    assert_eq!(app.clock.calls() - clock_before, 1);
    assert_eq!(app.ids.calls() - ids_before, 1);
    assert_eq!(policy.calls(), 1);
}

#[test]
fn peers_created_before_finish_share_the_closed_lifecycle_gate() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let mut app = support::app_with_policy(Arc::new(policy.clone()));
    let mut peer = app.peer();
    app.finish(ShutdownReason::UserQuit).unwrap();
    let receipts_before = app.count_rows("command_receipts");
    let events_before = app.count_rows("event_stream");
    let projection_before = app.persisted_last_sequence();
    let approvals_before = app.count_rows("approval_records");
    let clock_before = app.clock.calls();
    let ids_before = app.ids.calls();
    let policy_before = policy.calls();

    assert_eq!(
        peer.execute_user(ApplicationCommand::ShowStatus).unwrap_err(),
        AppError::LifecycleFinished
    );
    assert_eq!(app.count_rows("command_receipts"), receipts_before);
    assert_eq!(app.count_rows("event_stream"), events_before);
    assert_eq!(app.persisted_last_sequence(), projection_before);
    assert_eq!(app.count_rows("approval_records"), approvals_before);
    assert_eq!(app.clock.calls(), clock_before);
    assert_eq!(app.ids.calls(), ids_before);
    assert_eq!(policy.calls(), policy_before);
}

#[test]
fn transaction_rejects_a_closed_authoritative_session_before_dependencies_or_writes() {
    let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
    let app = support::app_with_policy(Arc::new(policy.clone()));
    let mut peer = app.peer();
    app.mark_current_session_finished_in_database();
    let receipts_before = app.count_rows("command_receipts");
    let events_before = app.count_rows("event_stream");
    let projection_before = app.persisted_last_sequence();
    let approvals_before = app.count_rows("approval_records");
    let clock_before = app.clock.calls();
    let ids_before = app.ids.calls();
    let policy_before = policy.calls();

    assert_eq!(
        peer.execute(envelope(1_600, 1_601, ApplicationCommand::ShowStatus))
            .unwrap_err(),
        AppError::LifecycleFinished
    );
    assert_eq!(app.count_rows("command_receipts"), receipts_before);
    assert_eq!(app.count_rows("event_stream"), events_before);
    assert_eq!(app.persisted_last_sequence(), projection_before);
    assert_eq!(app.count_rows("approval_records"), approvals_before);
    assert_eq!(app.clock.calls(), clock_before);
    assert_eq!(app.ids.calls(), ids_before);
    assert_eq!(policy.calls(), policy_before);
}

#[test]
fn zero_event_decisions_replay_and_conflict_without_dependencies_or_effects() {
    for (offset, decision) in [
        AuthorizationDecision::Denied(PolicyDecision::Denied),
        AuthorizationDecision::ApprovalRequired,
    ]
    .into_iter()
    .enumerate()
    {
        let policy = support::RecordingPolicy::new(decision);
        let mut app = support::app_with_policy(Arc::new(policy.clone()));
        let id = 1_700 + offset as u128;
        let command = envelope(id, id + 10, ApplicationCommand::ShowStatus);
        let events_before = app.count_rows("event_stream");
        let projection_before = app.persisted_last_sequence();
        let approvals_before = app.count_rows("approval_records");
        let clock_before = app.clock.calls();
        let ids_before = app.ids.calls();
        let first = app.execute(command.clone()).unwrap_err();

        policy.set_decision(AuthorizationDecision::Granted);
        let policy_before_replay = policy.calls();
        assert_eq!(app.execute(command).unwrap_err(), first);
        assert_eq!(
            app.execute(envelope(id, id + 10, ApplicationCommand::ShowHelp))
                .unwrap_err(),
            AppError::CommandConflict
        );
        assert_eq!(app.count_rows("command_receipts"), 1);
        assert_eq!(app.count_rows("command_event_refs"), 0);
        assert_eq!(app.count_rows("event_stream"), events_before);
        assert_eq!(app.persisted_last_sequence(), projection_before);
        assert_eq!(app.count_rows("approval_records"), approvals_before);
        assert_eq!(app.clock.calls(), clock_before);
        assert_eq!(app.ids.calls(), ids_before);
        assert_eq!(policy.calls(), policy_before_replay);
    }
}

#[test]
fn concurrent_zero_event_same_and_conflicting_commands_have_one_receipt_authority() {
    for (case, decision) in [
        AuthorizationDecision::Denied(PolicyDecision::Denied),
        AuthorizationDecision::ApprovalRequired,
    ]
    .into_iter()
    .enumerate()
    {
        let policy = support::RecordingPolicy::new(decision);
        let app = support::app_with_policy(Arc::new(policy.clone()));
        let mut first = app.peer();
        let mut second = app.peer();
        let command = envelope(1_800 + case as u128, 1_810, ApplicationCommand::ShowStatus);
        let barrier = Arc::new(Barrier::new(3));
        let first_barrier = barrier.clone();
        let first_command = command.clone();
        let first_handle = thread::spawn(move || {
            first_barrier.wait();
            first.execute(first_command)
        });
        let second_barrier = barrier.clone();
        let second_handle = thread::spawn(move || {
            second_barrier.wait();
            second.execute(command)
        });
        let events_before = app.count_rows("event_stream");
        let projection_before = app.persisted_last_sequence();
        let clock_before = app.clock.calls();
        let ids_before = app.ids.calls();
        barrier.wait();
        let results = [first_handle.join().unwrap(), second_handle.join().unwrap()];

        assert_eq!(results[0].as_ref().unwrap_err(), results[1].as_ref().unwrap_err());
        assert_eq!(app.count_rows("command_receipts"), 1);
        assert_eq!(app.count_rows("command_event_refs"), 0);
        assert_eq!(app.count_rows("event_stream"), events_before);
        assert_eq!(app.persisted_last_sequence(), projection_before);
        assert_eq!(app.count_rows("approval_records"), 0);
        assert_eq!(app.clock.calls(), clock_before);
        assert_eq!(app.ids.calls(), ids_before);
        assert_eq!(policy.calls(), 1);

        let policy = support::RecordingPolicy::new(decision);
        let app = support::app_with_policy(Arc::new(policy.clone()));
        let mut first = app.peer();
        let mut second = app.peer();
        let barrier = Arc::new(Barrier::new(3));
        let first_barrier = barrier.clone();
        let first_handle = thread::spawn(move || {
            first_barrier.wait();
            first.execute(envelope(
                1_900 + case as u128,
                1_910,
                ApplicationCommand::ShowHelp,
            ))
        });
        let second_barrier = barrier.clone();
        let second_handle = thread::spawn(move || {
            second_barrier.wait();
            second.execute(envelope(
                1_900 + case as u128,
                1_911,
                ApplicationCommand::ShowStatus,
            ))
        });
        let events_before = app.count_rows("event_stream");
        let projection_before = app.persisted_last_sequence();
        let clock_before = app.clock.calls();
        let ids_before = app.ids.calls();
        barrier.wait();
        let results = [first_handle.join().unwrap(), second_handle.join().unwrap()];

        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(AppError::CommandConflict)))
                .count(),
            1
        );
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 2);
        assert_eq!(app.count_rows("command_receipts"), 1);
        assert_eq!(app.count_rows("command_event_refs"), 0);
        assert_eq!(app.count_rows("event_stream"), events_before);
        assert_eq!(app.persisted_last_sequence(), projection_before);
        assert_eq!(app.count_rows("approval_records"), 0);
        assert_eq!(app.clock.calls(), clock_before);
        assert_eq!(app.ids.calls(), ids_before);
        assert_eq!(policy.calls(), 1);
    }
}

#[test]
fn zero_event_receipt_failures_roll_back_without_consuming_dependencies() {
    for (case, decision) in [
        AuthorizationDecision::Denied(PolicyDecision::Denied),
        AuthorizationDecision::ApprovalRequired,
    ]
    .into_iter()
    .enumerate()
    {
        for failure in [
            support::HookFailure::OutcomeMaterialization,
            support::HookFailure::ReceiptWrite,
        ] {
            let policy = support::RecordingPolicy::new(decision);
            let hook = Arc::new(support::TestCommandHook::failing(failure));
            let mut app = support::app_with_policy_and_hook(Arc::new(policy.clone()), hook);
            let events_before = app.count_rows("event_stream");
            let projection_before = app.persisted_last_sequence();
            let approvals_before = app.count_rows("approval_records");
            let clock_before = app.clock.calls();
            let ids_before = app.ids.calls();

            assert_eq!(
                app.execute(envelope(
                    2_000 + (case * 10) as u128 + failure as u128,
                    2_100,
                    ApplicationCommand::ShowStatus,
                ))
                .unwrap_err(),
                AppError::Persistence(PersistenceError::QueryFailed)
            );
            assert_eq!(app.count_rows("command_receipts"), 0);
            assert_eq!(app.count_rows("command_event_refs"), 0);
            assert_eq!(app.count_rows("event_stream"), events_before);
            assert_eq!(app.persisted_last_sequence(), projection_before);
            assert_eq!(app.count_rows("approval_records"), approvals_before);
            assert_eq!(app.clock.calls(), clock_before);
            assert_eq!(app.ids.calls(), ids_before);
            assert_eq!(policy.calls(), 1);
        }
    }
}

#[test]
fn every_durable_receipt_boundary_is_validated_before_dependencies_or_mutation() {
    for (offset, tamper) in [
        support::ReceiptTamper::NoncanonicalRequest,
        support::ReceiptTamper::NoncanonicalOutcome,
        support::ReceiptTamper::TypedInvalidRequest,
        support::ReceiptTamper::TypedInvalidOutcome,
        support::ReceiptTamper::FingerprintMismatch,
        support::ReceiptTamper::CapabilityMismatch,
        support::ReceiptTamper::PolicyDecisionMismatch,
        support::ReceiptTamper::EventRefOutcomeMismatch,
        support::ReceiptTamper::OrdinalGap,
        support::ReceiptTamper::MalformedReference,
    ]
    .into_iter()
    .enumerate()
    {
        let policy = support::RecordingPolicy::new(AuthorizationDecision::Granted);
        let mut app = support::app_with_policy(Arc::new(policy.clone()));
        let command = envelope(
            2_200 + offset as u128,
            2_300 + offset as u128,
            ApplicationCommand::ShowStatus,
        );
        app.execute(command.clone()).unwrap();
        app.tamper_receipt(command.command_id, tamper);
        let receipt_before = app.receipt_row(command.command_id);
        let refs_before = app.event_ref_rows(command.command_id);
        let events_before = app.count_rows("event_stream");
        let projection_before = app.persisted_last_sequence();
        let approvals_before = app.count_rows("approval_records");
        let clock_before = app.clock.calls();
        let ids_before = app.ids.calls();
        let policy_before = policy.calls();

        assert_eq!(
            app.execute(command.clone()).unwrap_err(),
            AppError::Persistence(PersistenceError::InvalidEventRecord),
            "tamper {tamper:?}"
        );
        assert_eq!(app.receipt_row(command.command_id), receipt_before);
        assert_eq!(app.event_ref_rows(command.command_id), refs_before);
        assert_eq!(app.count_rows("event_stream"), events_before);
        assert_eq!(app.persisted_last_sequence(), projection_before);
        assert_eq!(app.count_rows("approval_records"), approvals_before);
        assert_eq!(app.clock.calls(), clock_before);
        assert_eq!(app.ids.calls(), ids_before);
        assert_eq!(policy.calls(), policy_before);
    }
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
    let (_, request_json, _, _, outcome_json) = app.receipt_row(outcome.command_id);
    assert!(!request_json.contains("super-secret-value"));
    assert!(!outcome_json.contains("super-secret-value"));
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
