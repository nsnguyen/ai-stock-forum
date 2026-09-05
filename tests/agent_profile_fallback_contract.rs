mod support;

use std::{
    io::{self, BufRead, Cursor, Read, Write},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use ai_stock_forum::{
    agents::{
        AgentProfileDraft, AgentProfileVersion, AgentReadiness, AgentRole, ProfileEditPreview,
        builtin_profile_templates, diff_profile,
    },
    app::{
        AgentProfileCreatedView, AgentProfileHistoryEntry, AgentProfileHistoryView,
        AgentProfileSummary, AgentProfileView, AgentProfileVersionActivatedView, AppError,
        ApplicationCommand, CommandOutcome, CommandView, HelpView, ShutdownDisposition,
        ShutdownReason,
    },
    domain::{
        AgentProfileId, AgentProfileVersionId, CommandId, CorrelationId, MemoryNamespaceId,
        ObjectVersion, ProfileReviewToken, sha256,
    },
    runtime::{ApplicationRuntime, CommandExecutor},
    ui::command::{FallbackParsedLine, FallbackRunner, TextRenderer, parse_fallback_line},
};
use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use uuid::Uuid;

const WAIT: Duration = Duration::from_secs(5);

fn run_script(
    client: ai_stock_forum::runtime::RuntimeClient,
    script: impl AsRef<[u8]>,
) -> (ShutdownReason, String) {
    let mut output = Vec::new();
    let reason = FallbackRunner::new(client, false)
        .run(Cursor::new(script.as_ref()), &mut output)
        .unwrap();
    (reason, String::from_utf8(output).unwrap())
}

fn create_profile(
    client: &ai_stock_forum::runtime::RuntimeClient,
    name: &str,
) -> ai_stock_forum::app::AgentProfileCreatedView {
    let template = &builtin_profile_templates()[0];
    let mut draft = template.copy_to_draft().unwrap();
    draft.display_name = name.to_owned();
    let outcome = client
        .submit(ApplicationCommand::CreateAgentProfile {
            draft,
            template_provenance: Some(template.provenance()),
        })
        .unwrap();
    let CommandView::AgentProfileCreated(created) = outcome.view else {
        panic!("expected created profile view");
    };
    created
}

fn profile_count(client: &ai_stock_forum::runtime::RuntimeClient) -> usize {
    let outcome = client.submit(ApplicationCommand::ListAgentProfiles).unwrap();
    let CommandView::AgentProfiles(view) = outcome.view else {
        panic!("expected agent profile list");
    };
    view.profiles.len()
}

fn profile_history_len(
    client: &ai_stock_forum::runtime::RuntimeClient,
    profile_id: AgentProfileId,
) -> usize {
    let outcome = client
        .submit(ApplicationCommand::ShowAgentProfileHistory { profile_id })
        .unwrap();
    let CommandView::AgentProfileHistory(view) = outcome.view else {
        panic!("expected agent profile history");
    };
    view.versions.len()
}

fn edited_script(profile_id: AgentProfileId, confirmation: &str) -> String {
    format!("{}{confirmation}\n", edited_workflow(profile_id))
}

fn edited_workflow(profile_id: AgentProfileId) -> String {
    format!(
        "agent edit {profile_id}\n:role custom\n:next\nFallback Editor\n:next\nEdited in the fallback workflow.\n:next\nquality research\n:tag remove growth\n:tag remove catalysts\n:tag add quality\n:next\nCalm and exact.\n:next\nCite primary evidence.\n:next\n:provider local\n:model analyst-v2\n:next\n:review\n:activate\n"
    )
}

#[test]
fn parser_accepts_only_the_six_exact_agent_forms_and_typed_ids() {
    let id = "00000000-0000-0000-0000-00000000002a";
    assert!(matches!(
        parse_fallback_line(b"agent list"),
        FallbackParsedLine::Command(ApplicationCommand::ListAgentProfiles)
    ));
    assert!(matches!(
        parse_fallback_line(format!("agent show {id}").as_bytes()),
        FallbackParsedLine::Command(ApplicationCommand::ShowAgentProfile { .. })
    ));
    assert!(matches!(
        parse_fallback_line(format!("agent history {id}").as_bytes()),
        FallbackParsedLine::Command(ApplicationCommand::ShowAgentProfileHistory { .. })
    ));

    for accepted in [
        "agent create",
        "agent create builtin.bull",
        &format!("agent edit {id}"),
    ] {
        assert!(
            !matches!(
                parse_fallback_line(accepted.as_bytes()),
                FallbackParsedLine::Command(ApplicationCommand::RejectInput(_))
            ),
            "expected accepted agent form: {accepted}"
        );
    }

    for rejected in [
        "agent",
        "agent list extra",
        "agent show",
        "agent show not-a-uuid",
        "agent history not-a-uuid",
        "agent create unknown-template",
        "agent edit not-a-uuid",
        "agent edit 00000000-0000-0000-0000-00000000002a extra",
        "agent delete 00000000-0000-0000-0000-00000000002a",
    ] {
        assert!(
            matches!(
                parse_fallback_line(rejected.as_bytes()),
                FallbackParsedLine::Command(ApplicationCommand::RejectInput(_))
            ),
            "expected rejected agent form: {rejected}"
        );
    }
}

#[test]
fn create_selects_a_pinned_template_edits_fields_and_waits_for_yes() {
    let fixture = support::runtime();
    let client = fixture.client();

    let (_, selection) = run_script(client.clone(), "agent create\n:cancel\n");
    assert!(selection.contains("Profile templates:"));
    assert!(selection.contains("builtin.bull"));
    assert!(selection.contains("Profile creation cancelled."));
    assert_eq!(profile_count(&client), 0);

    let script = concat!(
        "agent create builtin.bull\n",
        ":role custom\n",
        ":next\n",
        "Fallback Analyst\n",
        ":next\n",
        "Edited description.\n",
        ":next\n",
        "special situations\n",
        ":tag remove growth\n",
        ":tag remove catalysts\n",
        ":tag add catalysts\n",
        ":next\n",
        "Patient and skeptical.\n",
        ":next\n",
        "Separate facts from estimates.\n",
        ":next\n",
        ":provider local\n",
        ":model analyst-v1\n",
        ":next\n",
        ":review\n",
        ":activate\n",
        "yes\n",
    );
    let (_, output) = run_script(client.clone(), script);
    assert!(output.contains("Create profile review"));
    let review = output
        .rsplit("Create profile review")
        .next()
        .expect("create review section is rendered");
    let positions = [
        "display_name",
        "description",
        "role",
        "primary_specialty",
        "specialty_tags",
        "personality",
        "instructions",
        "bindings",
    ]
    .map(|field| review.find(field).expect("create diff field is rendered"));
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(review.contains("Bull Researcher -> Fallback Analyst"));
    assert!(review.contains("role: bull -> custom"));
    assert!(review.contains("Unchanged fields omitted."));
    assert!(output.contains("Confirm activation? [y/yes or n/no]"));
    assert!(output.contains("Agent profile created:"));

    let listed = client.submit(ApplicationCommand::ListAgentProfiles).unwrap();
    let CommandView::AgentProfiles(listed) = listed.view else {
        panic!("expected list view");
    };
    assert_eq!(listed.profiles.len(), 1);
    assert_eq!(listed.profiles[0].display_name, "Fallback Analyst");
    assert_eq!(listed.profiles[0].role, AgentRole::Custom);
    fixture.finish_and_join(ShutdownReason::InputClosed);
}

#[test]
fn edit_loads_active_version_previews_ordered_diffs_and_no_returns_to_review() {
    let fixture = support::runtime();
    let client = fixture.client();
    let created = create_profile(&client, "Original Analyst");

    let (_, rejected) = run_script(client.clone(), edited_script(created.profile_id, "no"));
    assert!(rejected.contains("Original Analyst"));
    let names = [
        "display_name",
        "description",
        "role",
        "primary_specialty",
        "specialty_tags",
        "personality",
        "instructions",
        "bindings",
    ];
    let review = rejected
        .rsplit("Edit profile review")
        .next()
        .expect("edit review section is rendered");
    let positions = names
        .map(|name| review.find(name).expect("ordered diff field is rendered"));
    assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(rejected.contains("Activation declined; returned to review."));
    assert_eq!(profile_history_len(&client, created.profile_id), 1);

    let (_, accepted) = run_script(client.clone(), edited_script(created.profile_id, "y"));
    assert!(accepted.contains("Confirm activation? [y/yes or n/no]"));
    assert!(accepted.contains("Agent profile version activated:"));
    assert_eq!(profile_history_len(&client, created.profile_id), 2);
    fixture.finish_and_join(ShutdownReason::InputClosed);
}

#[test]
fn eof_cancel_and_editor_prose_create_no_durable_draft_or_generic_log_entry() {
    for script in [
        "agent create builtin.bear\n:next\nprivate draft prose\n",
        "agent create builtin.bear\n:next\nprivate draft prose\n:cancel\n",
    ] {
        let fixture = support::runtime();
        let client = fixture.client();
        let (_, output) = run_script(client.clone(), script);
        assert_eq!(profile_count(&client), 0);

        let (_, audit) = run_script(client.clone(), "/audit tail 100\n");
        assert!(!audit.contains("private draft prose"));
        assert!(!audit.contains("unknown command private"));
        assert!(output.contains("private draft prose") || script.ends_with(":cancel\n"));
        fixture.finish_and_join(ShutdownReason::InputClosed);
    }
}

#[test]
fn list_show_and_history_are_deterministic_terminal_safe_typed_views() {
    let fixture = support::runtime();
    let client = fixture.client();
    let created = create_profile(&client, "Unsafe\\nName");
    let script = format!(
        "agent list\nagent show {}\nagent history {}\n",
        created.profile_id, created.profile_id
    );

    let (_, first) = run_script(client.clone(), &script);
    let (_, second) = run_script(client.clone(), &script);
    assert_eq!(first, second);
    assert!(first.contains("NAME | ROLE | SPECIALTY | READY | VERSION | ID"));
    assert!(first.contains("Display name: Unsafe\\\\nName"));
    for label in [
        "Description:",
        "Role:",
        "Primary specialty:",
        "Specialty tags:",
        "Personality:",
        "Instructions:",
        "Template provenance:",
        "Bindings readiness:",
        "Skill refs: none",
        "MCP refs: none",
        "Memory namespace ID:",
        "Policy reference:",
        "Created time:",
        "Digest:",
        "VERSION | VERSION ID | PREDECESSOR | CREATED | READY | DIGEST",
    ] {
        assert!(first.contains(label), "missing typed field {label}");
    }
    assert!(!first.contains("{\""));
    fixture.finish_and_join(ShutdownReason::InputClosed);
}

#[test]
fn list_and_history_cap_rows_and_render_deterministic_omitted_counts() {
    let profiles = (0..102_u128)
        .map(|offset| AgentProfileSummary {
            profile_id: AgentProfileId::from_uuid(Uuid::from_u128(1_000 + offset)),
            profile_version_id: AgentProfileVersionId::from_uuid(Uuid::from_u128(2_000 + offset)),
            version: ObjectVersion::new(1).unwrap(),
            display_name: format!("Profile {offset:03}"),
            role: AgentRole::Custom,
            primary_specialty: "research".to_owned(),
            readiness: AgentReadiness::NotReady,
            content_digest: sha256(&offset.to_be_bytes()),
        })
        .collect::<Vec<_>>();
    let mut list = Vec::new();
    TextRenderer::render_view(
        &CommandView::AgentProfiles(ai_stock_forum::app::AgentProfilesView { profiles }),
        &mut list,
    )
    .unwrap();
    let list = String::from_utf8(list).unwrap();
    assert_eq!(list.lines().filter(|line| line.contains(" | ")).count(), 101);
    assert!(list.contains("... 2 profiles omitted."));
    assert!(list.contains("Profile 000"));
    assert!(list.contains("Profile 099"));
    assert!(!list.contains("Profile 100"));

    let versions = (0..102_u128)
        .map(|offset| AgentProfileHistoryEntry {
            profile_version_id: AgentProfileVersionId::from_uuid(Uuid::from_u128(3_000 + offset)),
            version: ObjectVersion::new(u64::try_from(offset + 1).unwrap()).unwrap(),
            supersedes: (offset > 0).then(|| {
                AgentProfileVersionId::from_uuid(Uuid::from_u128(2_999 + offset))
            }),
            created_at_ms: i64::try_from(offset).unwrap(),
            readiness: AgentReadiness::Ready,
            content_digest: sha256(&offset.to_le_bytes()),
        })
        .collect::<Vec<_>>();
    let mut history = Vec::new();
    TextRenderer::render_view(
        &CommandView::AgentProfileHistory(AgentProfileHistoryView {
            profile_id: AgentProfileId::from_uuid(Uuid::from_u128(4_000)),
            active_version_id: AgentProfileVersionId::from_uuid(Uuid::from_u128(4_001)),
            versions,
        }),
        &mut history,
    )
    .unwrap();
    let history = String::from_utf8(history).unwrap();
    assert_eq!(history.lines().filter(|line| line.contains(" | ")).count(), 101);
    assert!(history.contains("... 2 versions omitted."));
}

#[derive(Default)]
struct WorkflowObservations {
    creates: usize,
    activations: usize,
    cancellations: usize,
}

struct WorkflowExecutor {
    profile: AgentProfileVersion,
    entered: Sender<()>,
    release: Receiver<()>,
    observations: Arc<Mutex<WorkflowObservations>>,
}

impl CommandExecutor for WorkflowExecutor {
    fn execute_user(&mut self, command: ApplicationCommand) -> Result<CommandOutcome, AppError> {
        match command {
            ApplicationCommand::ShowHelp => {
                self.entered.send(()).unwrap();
                self.release.recv().unwrap();
                Ok(workflow_outcome(CommandView::Help(HelpView)))
            }
            ApplicationCommand::ShowAgentProfile { .. } => {
                let readiness = match (
                    &self.profile.bindings().model_provider,
                    &self.profile.bindings().model_name,
                ) {
                    (Some(_), Some(_)) => AgentReadiness::Ready,
                    _ => AgentReadiness::NotReady,
                };
                Ok(workflow_outcome(CommandView::AgentProfile(
                    AgentProfileView {
                        profile: self.profile.clone(),
                        readiness,
                    },
                )))
            }
            ApplicationCommand::CreateAgentProfile { .. } => {
                self.observations.lock().unwrap().creates += 1;
                Ok(workflow_outcome(CommandView::AgentProfileCreated(
                    AgentProfileCreatedView {
                        profile_id: AgentProfileId::from_uuid(Uuid::from_u128(8_000)),
                        profile_version_id: AgentProfileVersionId::from_uuid(Uuid::from_u128(8_001)),
                        version: ObjectVersion::new(1).unwrap(),
                        readiness: AgentReadiness::Ready,
                    },
                )))
            }
            ApplicationCommand::ActivateAgentProfileVersion { .. } => {
                self.observations.lock().unwrap().activations += 1;
                Ok(workflow_outcome(
                    CommandView::AgentProfileVersionActivated(AgentProfileVersionActivatedView {
                        profile_id: self.profile.profile_id(),
                        profile_version_id: AgentProfileVersionId::from_uuid(Uuid::from_u128(8_002)),
                        previous_version_id: self.profile.profile_version_id(),
                        version: ObjectVersion::new(2).unwrap(),
                        readiness: AgentReadiness::Ready,
                    }),
                ))
            }
            _ => Ok(workflow_outcome(CommandView::Help(HelpView))),
        }
    }

    fn preview_agent_profile_edit(
        &mut self,
        profile_id: AgentProfileId,
        expected_active_version_id: AgentProfileVersionId,
        candidate: AgentProfileDraft,
    ) -> Result<ProfileEditPreview, AppError> {
        Ok(ProfileEditPreview {
            profile_id,
            expected_active_version_id,
            diffs: diff_profile(&self.profile, &candidate).unwrap(),
            review_token: ProfileReviewToken::from_uuid(Uuid::from_u128(8_003)),
            review_digest: sha256(b"review"),
        })
    }

    fn cancel_agent_profile_edit(&mut self) {
        self.observations.lock().unwrap().cancellations += 1;
    }

    fn finish(&mut self, _reason: ShutdownReason) -> Result<(), AppError> {
        Ok(())
    }
}

fn workflow_profile() -> AgentProfileVersion {
    let template = &builtin_profile_templates()[0];
    let mut draft = template.copy_to_draft().unwrap();
    draft.display_name = "Original Backpressure Analyst".to_owned();
    AgentProfileVersion::create(
        AgentProfileId::from_uuid(Uuid::from_u128(7_000)),
        AgentProfileVersionId::from_uuid(Uuid::from_u128(7_001)),
        MemoryNamespaceId::from_uuid(Uuid::from_u128(7_002)),
        1_700_000_000_000,
        draft,
        Some(template.provenance()),
    )
    .unwrap()
}

fn workflow_outcome(view: CommandView) -> CommandOutcome {
    CommandOutcome {
        command_id: CommandId::from_uuid(Uuid::from_u128(9_000)),
        correlation_id: CorrelationId::from_uuid(Uuid::from_u128(9_001)),
        committed_events: Vec::new(),
        view,
        shutdown: ShutdownDisposition::Continue,
    }
}

struct ChannelReader {
    input: Receiver<Option<Vec<u8>>>,
    buffer: Vec<u8>,
    position: usize,
}

impl Read for ChannelReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let available = self.fill_buf()?;
        let amount = available.len().min(output.len());
        output[..amount].copy_from_slice(&available[..amount]);
        self.consume(amount);
        Ok(amount)
    }
}

impl BufRead for ChannelReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.position == self.buffer.len() {
            match self.input.recv() {
                Ok(Some(buffer)) => {
                    self.buffer = buffer;
                    self.position = 0;
                }
                Ok(None) | Err(_) => {
                    self.buffer.clear();
                    self.position = 0;
                }
            }
        }
        Ok(&self.buffer[self.position..])
    }

    fn consume(&mut self, amount: usize) {
        self.position = (self.position + amount).min(self.buffer.len());
    }
}

#[derive(Clone)]
struct ObservedWriter {
    output: Arc<Mutex<Vec<u8>>>,
    changed: Sender<()>,
}

impl Write for ObservedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.output.lock().unwrap().extend_from_slice(bytes);
        let _ = self.changed.send(());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn wait_for_output(output: &Arc<Mutex<Vec<u8>>>, changed: &Receiver<()>, needle: &str) {
    loop {
        if String::from_utf8_lossy(&output.lock().unwrap()).contains(needle) {
            return;
        }
        changed
            .recv_timeout(WAIT)
            .unwrap_or_else(|_| panic!("timed out waiting for output: {needle}"));
    }
}

struct WorkflowHarness {
    runtime: ApplicationRuntime,
    input: Sender<Option<Vec<u8>>>,
    output: Arc<Mutex<Vec<u8>>>,
    changed: Receiver<()>,
    entered: Receiver<()>,
    release: Sender<()>,
    runner: thread::JoinHandle<Result<ShutdownReason, ai_stock_forum::ui::command::UiError>>,
    observations: Arc<Mutex<WorkflowObservations>>,
}

impl WorkflowHarness {
    fn new() -> Self {
        let (entered_sender, entered) = bounded(1);
        let (release, release_receiver) = bounded(1);
        let observations = Arc::new(Mutex::new(WorkflowObservations::default()));
        let runtime = ApplicationRuntime::spawn(
            WorkflowExecutor {
                profile: workflow_profile(),
                entered: entered_sender,
                release: release_receiver,
                observations: observations.clone(),
            },
            1,
        )
        .unwrap();
        let (input, input_receiver) = unbounded();
        let (changed_sender, changed) = unbounded();
        let output = Arc::new(Mutex::new(Vec::new()));
        let reader = ChannelReader {
            input: input_receiver,
            buffer: Vec::new(),
            position: 0,
        };
        let writer = ObservedWriter {
            output: output.clone(),
            changed: changed_sender,
        };
        let client = runtime.client();
        let runner = thread::spawn(move || FallbackRunner::new(client, false).run(reader, writer));
        Self {
            runtime,
            input,
            output,
            changed,
            entered,
            release,
            runner,
            observations,
        }
    }

    fn send(&self, input: &str) {
        self.input.send(Some(input.as_bytes().to_vec())).unwrap();
    }

    fn wait(&self, needle: &str) {
        wait_for_output(&self.output, &self.changed, needle);
    }

    fn saturate(
        &self,
    ) -> (
        ai_stock_forum::runtime::PendingOutcome,
        ai_stock_forum::runtime::PendingOutcome,
    ) {
        let blocked = self
            .runtime
            .client()
            .try_submit(ApplicationCommand::ShowHelp)
            .unwrap();
        self.entered.recv_timeout(WAIT).unwrap();
        let queued = self
            .runtime
            .client()
            .try_submit(ApplicationCommand::ShowStatus)
            .unwrap();
        (blocked, queued)
    }

    fn release_queue(
        &self,
        blocked: ai_stock_forum::runtime::PendingOutcome,
        queued: ai_stock_forum::runtime::PendingOutcome,
    ) {
        self.release.send(()).unwrap();
        blocked.recv().unwrap();
        queued.recv().unwrap();
    }

    fn finish(self) -> Arc<Mutex<WorkflowObservations>> {
        self.input.send(None).unwrap();
        assert_eq!(
            self.runner.join().unwrap().unwrap(),
            ShutdownReason::InputClosed
        );
        self.runtime
            .finish_and_join(ShutdownReason::InputClosed)
            .unwrap();
        self.observations
    }
}

#[test]
fn create_confirmation_survives_backpressure_and_yes_retries_successfully() {
    let harness = WorkflowHarness::new();
    harness.send(concat!(
        "agent create builtin.custom\n:next\nRetry Create\n:next\nDescription\n:next\nresearch\n:next\nPatient\n:next\nCite evidence\n:next\n:provider local\n:model model\n:next\n:review\n:activate\n"
    ));
    harness.wait("Confirm activation? [y/yes or n/no]");
    let (blocked, queued) = harness.saturate();
    harness.send("yes\n");
    harness.wait("Command queue is busy; try again.");
    harness.release_queue(blocked, queued);
    harness.send("y\n");
    harness.wait("Agent profile created:");
    let observations = harness.finish();
    assert_eq!(observations.lock().unwrap().creates, 1);
}

#[test]
fn edit_confirmation_survives_backpressure_and_cancel_cleans_service_review() {
    let harness = WorkflowHarness::new();
    harness.send(&edited_workflow(workflow_profile().profile_id()));
    harness.wait("Confirm activation? [y/yes or n/no]");
    let (blocked, queued) = harness.saturate();
    harness.send("yes\n");
    harness.wait("Command queue is busy; try again.");
    harness.send(":cancel\n");
    harness.release_queue(blocked, queued);
    harness.wait("Profile edit cancelled.");
    let observations = harness.finish();
    let observations = observations.lock().unwrap();
    assert_eq!(observations.activations, 0);
    assert_eq!(observations.cancellations, 1);
}

#[allow(dead_code)]
fn _draft_type_is_part_of_the_contract(_: AgentProfileDraft) {}
