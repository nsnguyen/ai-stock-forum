use super::{ApplicationCommand, CommandOutcome, CommandView, RuntimeError, TuiError, TuiRunner};
use crate::{
    audit::AuditEntry,
    skills::SkillVersionRef,
    ui::tui::{
        controller::merge_committed_audit,
        model::{RuntimeStatus, Severity, SkillsPane, TuiModel},
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SkillPreviewTarget {
    skill_ref: SkillVersionRef,
    starter: bool,
}

impl SkillPreviewTarget {
    fn selected(model: &TuiModel, index: usize, starter: bool) -> Option<Self> {
        let target = Self {
            skill_ref: model.skills.library.skills.get(index)?.skill_ref.clone(),
            starter,
        };
        target.is_current(model).then_some(target)
    }

    fn is_current(&self, model: &TuiModel) -> bool {
        if !model.skills.active
            || model.runtime_status == RuntimeStatus::Stopping
            || model.skills.pending_confirmation.is_some()
        {
            return false;
        }
        let index = if self.starter {
            if model.skills.pane != SkillsPane::CreateSource {
                return false;
            }
            let Some(index) = model.skills.selected_create_source.checked_sub(1) else {
                return false;
            };
            index
        } else {
            if !matches!(model.skills.pane, SkillsPane::List | SkillsPane::Detail)
                || model.skills.version_detail.is_some()
            {
                return false;
            }
            model.skills.selected_skill
        };
        model
            .skills
            .library
            .skills
            .get(index)
            .is_some_and(|summary| summary.skill_ref == self.skill_ref)
    }
}

impl TuiRunner {
    pub(super) fn queue_missing_skill_preview(&mut self) {
        if !self.model.skills.library_loaded {
            return;
        }
        if self.model.skills.pane == SkillsPane::CreateSource {
            if self.model.skills.create_source_detail.is_none()
                && let Some(index) = self.model.skills.selected_create_source.checked_sub(1)
            {
                self.queue_skill_preview(index, true);
            }
        } else if self.model.skills.detail.is_none() {
            self.queue_skill_preview(self.model.skills.selected_skill, false);
        }
    }

    pub(super) fn queue_skill_preview(&mut self, index: usize, starter: bool) {
        let target = SkillPreviewTarget::selected(&self.model, index, starter);
        self.deferred_skill_preview =
            target.filter(|target| self.pending_skill_preview.as_ref() != Some(target));
    }

    pub(super) fn start_skill_preview(
        &mut self,
        target: SkillPreviewTarget,
    ) -> Result<(), TuiError> {
        debug_assert!(self.pending.is_none());
        if !target.is_current(&self.model) {
            return Ok(());
        }
        let command = ApplicationCommand::ShowSkillVersion {
            selector: target.skill_ref.skill_id().into(),
            version: target.skill_ref.version(),
        };
        match self.client.try_submit(command) {
            Ok(pending) => {
                self.pending = Some(pending);
                self.pending_skill_preview = Some(target);
                self.model.set_command_in_flight_preserving_navigation();
                Ok(())
            }
            Err(RuntimeError::Backpressure) => {
                self.skill_preview_unavailable(&target);
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    pub(super) fn complete_skill_preview(
        &mut self,
        target: SkillPreviewTarget,
        outcome: CommandOutcome,
    ) {
        self.model.set_command_in_flight(false);
        // Keep the audit trail, but never apply a passive response through a
        // navigation-changing outcome path or restore an obsolete draft snapshot.
        merge_committed_audit(
            &mut self.model,
            outcome
                .committed_events
                .iter()
                .map(AuditEntry::from_event)
                .collect(),
        );
        if !target.is_current(&self.model) {
            return;
        }
        match outcome.view {
            CommandView::SkillVersion(detail) if detail.skill_ref == target.skill_ref => {
                if target.starter {
                    self.model.skills.create_source_detail = Some(detail);
                } else {
                    self.model.skills.detail = Some(detail);
                }
            }
            _ => self.skill_preview_unavailable(&target),
        }
    }

    pub(super) fn skill_preview_unavailable(&mut self, target: &SkillPreviewTarget) {
        if target.is_current(&self.model) {
            self.model.set_message(
                Severity::Warning,
                "Preview unavailable. Keep browsing or press Enter to retry.",
            );
        }
    }
}
