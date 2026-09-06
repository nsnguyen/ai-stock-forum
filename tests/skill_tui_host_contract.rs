use std::sync::{Arc, Mutex};

use ai_stock_forum::{
    app::{
        AppError, ApplicationCommand, CommandOutcome, PresentationSnapshot, ShutdownReason,
    },
    domain::{
        AgentProfileId, AgentProfileVersionId, InstallationId, SessionId, SkillId,
        SkillReviewToken, SkillVersionId, sha256,
    },
    runtime::{ApplicationRuntime, CommandExecutor},
    setup::SetupStatus,
    skills::{SkillDraft, SkillEditPreview},
    ui::{
        skill_editor::{SkillEditor, SkillEditorEffect, SkillPreviewRequest},
        tui::{
            ControllerEffect, execute_skill_effect,
            model::{SkillsPane, TuiModel},
        },
    },
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Create,
    Version(SkillId, SkillVersionId, SkillDraft),
    Assign,
    Upgrade,
    Unassign,
    Cancel,
    Finish,
}

struct RouteRecorder {
    calls: Arc<Mutex<Vec<Call>>>,
}

impl CommandExecutor for RouteRecorder {
    fn execute_user(&mut self, _command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        Err(AppError::LifecycleFinished)
    }

    fn preview_skill_creation(
        &mut self,
        _candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        self.calls.lock().unwrap().push(Call::Create);
        Err(AppError::SkillReviewUnavailable)
    }

    fn preview_skill_version(
        &mut self,
        skill_id: SkillId,
        expected_active_version_id: SkillVersionId,
        candidate: SkillDraft,
    ) -> Result<SkillEditPreview, AppError> {
        self.calls.lock().unwrap().push(Call::Version(
            skill_id,
            expected_active_version_id,
            candidate.clone(),
        ));
        Ok(SkillEditPreview {
            skill_id,
            expected_active_version_id: Some(expected_active_version_id),
            candidate_digest: sha256(b"candidate"),
            review_token: SkillReviewToken::from_uuid(Uuid::from_u128(90)),
            review_digest: sha256(b"review"),
        })
    }

    fn preview_agent_skill_assignment(
        &mut self,
        _profile_id: AgentProfileId,
        _expected_active_profile_version_id: AgentProfileVersionId,
        _skill: ai_stock_forum::skills::SkillVersionRef,
    ) -> Result<ai_stock_forum::app::AgentSkillAssignmentPreview, AppError> {
        self.calls.lock().unwrap().push(Call::Assign);
        Err(AppError::SkillReviewUnavailable)
    }

    fn preview_agent_skill_upgrade(
        &mut self,
        _profile_id: AgentProfileId,
        _expected_active_profile_version_id: AgentProfileVersionId,
        _expected: ai_stock_forum::skills::SkillVersionRef,
        _replacement: ai_stock_forum::skills::SkillVersionRef,
    ) -> Result<ai_stock_forum::app::AgentSkillAssignmentPreview, AppError> {
        self.calls.lock().unwrap().push(Call::Upgrade);
        Err(AppError::SkillReviewUnavailable)
    }

    fn preview_agent_skill_unassignment(
        &mut self,
        _profile_id: AgentProfileId,
        _expected_active_profile_version_id: AgentProfileVersionId,
        _expected: ai_stock_forum::skills::SkillVersionRef,
    ) -> Result<ai_stock_forum::app::AgentSkillAssignmentPreview, AppError> {
        self.calls.lock().unwrap().push(Call::Unassign);
        Err(AppError::SkillReviewUnavailable)
    }

    fn cancel_skill_review(&mut self) -> Result<(), AppError> {
        self.calls.lock().unwrap().push(Call::Cancel);
        Ok(())
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        self.calls.lock().unwrap().push(Call::Finish);
        Ok(())
    }
}

fn draft() -> SkillDraft {
    SkillDraft::new(
        "Version Route".to_owned(),
        "Purpose".to_owned(),
        "Use when routing previews.".to_owned(),
        Vec::new(),
        "Keep transport typed.".to_owned(),
        Vec::new(),
    )
    .unwrap()
}

fn model() -> TuiModel {
    TuiModel::new(
        PresentationSnapshot {
            installation_id: InstallationId::from_uuid(Uuid::from_u128(1)),
            session_id: SessionId::from_uuid(Uuid::from_u128(2)),
            database_readiness: ai_stock_forum::app::DatabaseReadiness::Ready,
            process_guard_ownership: ai_stock_forum::app::ProcessGuardOwnership::Held,
            setup_status: SetupStatus::NotStarted,
            recent_audit: Vec::new(),
            agent_profiles: ai_stock_forum::app::AgentProfilesView {
                profiles: Vec::new(),
                total_count: 0,
                returned_count: 0,
                truncated: false,
            },
            selected_agent_profile: None,
            selected_agent_profile_history: None,
        },
        false,
    )
}

#[test]
fn version_preview_round_trips_through_its_own_runtime_route_and_cleanup_is_once() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let runtime = ApplicationRuntime::spawn(
        RouteRecorder {
            calls: calls.clone(),
        },
        4,
    )
    .unwrap();
    let skill_id = SkillId::from_uuid(Uuid::from_u128(70));
    let version_id = SkillVersionId::from_uuid(Uuid::from_u128(71));
    let mut editor = SkillEditor::for_version(skill_id, version_id, draft());
    editor.go_to_review().unwrap();
    let SkillEditorEffect::Preview(request @ SkillPreviewRequest::Version { .. }) =
        editor.submit_keyboard_line("")
    else {
        panic!("version request")
    };
    let expected_candidate = request.candidate().clone();
    let mut model = model();
    model.skills.active = true;
    model.skills.pane = SkillsPane::Editor;
    model.skills.editor = Some(editor);

    execute_skill_effect(
        &runtime.client(),
        &mut model,
        ControllerEffect::RequestSkillPreview(request),
    )
    .expect("version preview route");

    assert!(model.skills.review_registered);
    assert!(model.skills.editor.as_ref().unwrap().review().is_some());
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        [Call::Version(skill_id, version_id, expected_candidate)]
    );

    execute_skill_effect(
        &runtime.client(),
        &mut model,
        ControllerEffect::CancelSkillReview,
    )
    .unwrap();
    execute_skill_effect(
        &runtime.client(),
        &mut model,
        ControllerEffect::CancelSkillReview,
    )
    .unwrap();
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|call| **call == Call::Cancel)
            .count(),
        1
    );

    runtime.finish_and_join(ShutdownReason::UserQuit).unwrap();
}

