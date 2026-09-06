use ai_stock_forum::{
    app::{
        AgentProfileSelector, AgentProfilesView, AppError, ApplicationCommand, ApplicationEvent,
    },
    domain::{AgentProfileId, ObjectVersion},
    policy::Capability,
};
use uuid::Uuid;

#[test]
fn profile_selectors_accept_uuid_or_canonical_display_name() {
    let id = AgentProfileId::from_uuid(Uuid::from_u128(41));
    assert_eq!(
        AgentProfileSelector::from_input(&id.to_string()).unwrap(),
        AgentProfileSelector::Id(id)
    );

    let selector = AgentProfileSelector::from_input("  Ma\u{00df}e\u{2003}Desk  ").unwrap();
    assert_eq!(selector.display_name(), Some("Ma\u{00df}e Desk"));
    assert_eq!(selector.normalized_name().unwrap().as_str(), "masse desk");
}

#[test]
fn command_contract_supports_selectors_and_exact_historical_versions() {
    let selector = AgentProfileSelector::from_input("Bull Researcher").unwrap();
    let show = ApplicationCommand::ShowAgentProfile {
        selector: selector.clone(),
    };
    let history = ApplicationCommand::ShowAgentProfileHistory {
        selector: selector.clone(),
    };
    let exact = ApplicationCommand::ShowAgentProfileVersion {
        selector,
        version: ObjectVersion::new(2).unwrap(),
    };

    assert!(matches!(show, ApplicationCommand::ShowAgentProfile { .. }));
    assert!(matches!(
        history,
        ApplicationCommand::ShowAgentProfileHistory { .. }
    ));
    assert!(matches!(
        exact,
        ApplicationCommand::ShowAgentProfileVersion { version, .. } if version.get() == 2
    ));
}

#[test]
fn policy_vocabulary_has_four_distinct_profile_capabilities() {
    let capabilities = [
        Capability::AgentProfileRead,
        Capability::AgentProfileCreate,
        Capability::AgentProfilePreview,
        Capability::AgentProfileActivate,
    ];
    let encoded = capabilities
        .into_iter()
        .map(|capability| serde_json::to_string(&capability).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(
        encoded,
        vec![
            "\"agent_profile_read\"",
            "\"agent_profile_create\"",
            "\"agent_profile_preview\"",
            "\"agent_profile_activate\"",
        ]
    );
}

#[test]
fn list_event_and_view_expose_only_bounded_metadata() {
    let event = ApplicationEvent::AgentProfilesListed {
        total_count: 151,
        returned_count: 100,
        truncated: true,
    };
    assert!(matches!(
        event,
        ApplicationEvent::AgentProfilesListed {
            total_count: 151,
            returned_count: 100,
            truncated: true,
        }
    ));

    let view = AgentProfilesView {
        profiles: Vec::new(),
        total_count: 151,
        returned_count: 100,
        truncated: true,
    };
    assert_eq!(view.returned_count, 100);
    assert!(view.truncated);
}

#[test]
fn history_mismatch_has_a_dedicated_safe_application_error() {
    let error = AppError::AgentProfileHistoryMismatch;
    assert_eq!(error.code(), "agent_profile_history_mismatch");
}
