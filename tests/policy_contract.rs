use ai_stock_forum::app::{ApplicationCommand, InputRejection, InputRejectionCategory, SafeToken};
use ai_stock_forum::domain::{sha256, Actor, ApprovalId, ObjectRef, ObjectVersion};
use ai_stock_forum::policy::{
    evaluate, ApprovalAction, ApprovalError, ApprovalRecord, ApprovalRecordBuilder,
    ApprovalResolution, ApprovalStatus, Capability, Effect, PolicyDecision, PolicyRule,
};
use serde_json::{json, Value};
use uuid::Uuid;

fn object() -> ObjectRef {
    ObjectRef::new(
        "git_commit",
        "commit-1",
        ObjectVersion::new(1).unwrap(),
        sha256(b"commit-1"),
    )
    .unwrap()
}

fn builder() -> ApprovalRecordBuilder {
    ApprovalRecord::builder(ApprovalAction::GitPush)
        .approval_id(ApprovalId::from_uuid(Uuid::from_u128(1)))
        .object(object())
        .actor(Actor::Human)
        .created_at_millis(100)
        .expires_at_millis(200)
}

fn valid_record() -> ApprovalRecord {
    builder().build().unwrap()
}

fn resolution(status: ApprovalStatus) -> ApprovalResolution {
    serde_json::from_value(json!({
        "status": match status {
            ApprovalStatus::Pending => "pending",
            ApprovalStatus::Approved => "approved",
            ApprovalStatus::Rejected => "rejected",
            ApprovalStatus::Expired => "expired",
        },
        "actor": "Human",
        "resolved_at_millis": 150,
    }))
    .unwrap()
}

fn record_json() -> Value {
    serde_json::to_value(valid_record()).unwrap()
}

#[test]
fn explicit_denial_wins_over_a_grant() {
    let rules = [
        PolicyRule::new(Effect::Grant, Capability::AuditRead),
        PolicyRule::new(Effect::Deny, Capability::AuditRead),
    ];

    assert_eq!(
        evaluate(Capability::AuditRead, &rules),
        PolicyDecision::Denied
    );
}

#[test]
fn missing_rule_denies_by_default() {
    assert_eq!(
        evaluate(Capability::GitPush, &[]),
        PolicyDecision::DeniedByDefault
    );
}

#[test]
fn denial_wins_when_it_precedes_a_grant() {
    let rules = [
        PolicyRule::new(Effect::Deny, Capability::AuditRead),
        PolicyRule::new(Effect::Grant, Capability::AuditRead),
    ];

    assert_eq!(
        evaluate(Capability::AuditRead, &rules),
        PolicyDecision::Denied
    );
}

#[test]
fn unrelated_rules_do_not_grant_a_capability() {
    let rules = [PolicyRule::new(Effect::Grant, Capability::GitPush)];

    assert_eq!(
        evaluate(Capability::AuditRead, &rules),
        PolicyDecision::DeniedByDefault
    );
}

#[test]
fn matching_grant_allows_a_capability_without_a_denial() {
    let rules = [PolicyRule::new(Effect::Grant, Capability::AuditRead)];

    assert_eq!(
        evaluate(Capability::AuditRead, &rules),
        PolicyDecision::Granted
    );
}

#[test]
fn commands_map_to_exact_safe_capabilities() {
    let commands = [
        (ApplicationCommand::ShowHelp, Capability::HelpRead),
        (ApplicationCommand::ShowStatus, Capability::StatusRead),
        (
            ApplicationCommand::ShowSetupStatus,
            Capability::SetupStatusRead,
        ),
        (
            ApplicationCommand::audit_tail(20).unwrap(),
            Capability::AuditRead,
        ),
        (
            ApplicationCommand::RejectInput(InputRejection::from_input(
                InputRejectionCategory::Malformed,
                Some(SafeToken::new("/not-supported").unwrap()),
                b"/not-supported",
            )),
            Capability::HelpRead,
        ),
        (ApplicationCommand::RequestShutdown, Capability::Shutdown),
    ];

    for (command, capability) in commands {
        assert_eq!(command.required_capability(), capability);
    }
}

#[test]
fn approval_requires_an_exact_object_and_pending_status() {
    assert_eq!(
        ApprovalRecord::builder(ApprovalAction::GitPush)
        .build()
        .unwrap_err(),
        ApprovalError::MissingApprovalId
    );
    assert!(!ApprovalStatus::Pending.is_terminal());
}

#[test]
fn approval_builder_constructs_a_valid_pending_record() {
    assert_eq!(
        serde_json::to_value(valid_record()).unwrap(),
        json!({
            "approval_id": "00000000-0000-0000-0000-000000000001",
            "action": "git_push",
            "object": {
                "kind": "git_commit",
                "id": "commit-1",
                "version": 1,
                "digest": sha256(b"commit-1").to_string(),
            },
            "actor": "Human",
            "status": "pending",
            "created_at_millis": 100,
            "expires_at_millis": 200,
            "resolution": null,
        })
    );
}

#[test]
fn approval_builder_rejects_each_missing_required_fact() {
    assert_eq!(
        ApprovalRecord::builder(ApprovalAction::GitPush)
            .object(object())
            .actor(Actor::Human)
            .created_at_millis(100)
            .build()
            .unwrap_err(),
        ApprovalError::MissingApprovalId
    );
    assert_eq!(
        ApprovalRecord::builder(ApprovalAction::GitPush)
            .approval_id(ApprovalId::from_uuid(Uuid::from_u128(1)))
            .actor(Actor::Human)
            .created_at_millis(100)
            .build()
            .unwrap_err(),
        ApprovalError::MissingObject
    );
    assert_eq!(
        ApprovalRecord::builder(ApprovalAction::GitPush)
            .approval_id(ApprovalId::from_uuid(Uuid::from_u128(1)))
            .object(object())
            .created_at_millis(100)
            .build()
            .unwrap_err(),
        ApprovalError::MissingActor
    );
    assert_eq!(
        ApprovalRecord::builder(ApprovalAction::GitPush)
            .approval_id(ApprovalId::from_uuid(Uuid::from_u128(1)))
            .object(object())
            .actor(Actor::Human)
            .build()
            .unwrap_err(),
        ApprovalError::MissingCreationTimestamp
    );
}

#[test]
fn approval_builder_rejects_non_pending_creation_and_invalid_expiry() {
    assert_eq!(
        builder()
            .status(ApprovalStatus::Approved)
            .build()
            .unwrap_err(),
        ApprovalError::InitialStatusMustBePending
    );
    assert_eq!(
        builder()
            .resolution(resolution(ApprovalStatus::Approved))
            .build()
            .unwrap_err(),
        ApprovalError::InitialResolutionNotAllowed
    );
    assert_eq!(
        builder()
            .expires_at_millis(100)
            .build()
            .unwrap_err(),
        ApprovalError::ExpiryMustFollowCreation
    );
    assert_eq!(
        builder()
            .expires_at_millis(99)
            .build()
            .unwrap_err(),
        ApprovalError::ExpiryMustFollowCreation
    );
}

#[test]
fn valid_approval_records_round_trip_through_serde() {
    let record = valid_record();
    let encoded = serde_json::to_string(&record).unwrap();

    assert_eq!(serde_json::from_str::<ApprovalRecord>(&encoded).unwrap(), record);
}

#[test]
fn serde_accepts_a_terminal_record_with_a_matching_terminal_resolution() {
    let mut encoded = record_json();
    encoded["status"] = json!("approved");
    encoded["resolution"] = serde_json::to_value(resolution(ApprovalStatus::Approved)).unwrap();

    let record = serde_json::from_value::<ApprovalRecord>(encoded.clone()).unwrap();
    assert_eq!(serde_json::to_value(record).unwrap(), encoded);
}

#[test]
fn serde_rejects_a_pending_record_with_a_resolution() {
    let mut encoded = record_json();
    encoded["resolution"] = serde_json::to_value(resolution(ApprovalStatus::Approved)).unwrap();

    assert!(serde_json::from_value::<ApprovalRecord>(encoded).is_err());
}

#[test]
fn serde_rejects_a_terminal_record_without_a_resolution() {
    let mut encoded = record_json();
    encoded["status"] = json!("approved");

    assert!(serde_json::from_value::<ApprovalRecord>(encoded).is_err());
}

#[test]
fn serde_rejects_a_terminal_record_with_a_mismatched_resolution() {
    let mut encoded = record_json();
    encoded["status"] = json!("approved");
    encoded["resolution"] = serde_json::to_value(resolution(ApprovalStatus::Rejected)).unwrap();

    assert!(serde_json::from_value::<ApprovalRecord>(encoded).is_err());
}

#[test]
fn serde_rejects_a_pending_resolution() {
    let mut encoded = record_json();
    encoded["status"] = json!("approved");
    encoded["resolution"] = json!({
        "status": "pending",
        "actor": "Human",
        "resolved_at_millis": 150,
    });

    assert!(serde_json::from_value::<ApprovalRecord>(encoded).is_err());
}

#[test]
fn serde_rejects_expiry_not_later_than_creation() {
    for expiry in [100, 99] {
        let mut encoded = record_json();
        encoded["expires_at_millis"] = json!(expiry);

        assert!(serde_json::from_value::<ApprovalRecord>(encoded).is_err());
    }
}
