# Two-pane TUI: approved direction and interaction specification

Status: the first implementation slice—the shared navigation/focus shell and
complete Agents path—is implemented. Full Memory and Skills usability passes
and Phase 3 remain future work. The manual `make dev` acceptance gate has not
been performed.

Source baseline: `5a24943c1de471363f81a1c50cad2d8fc740df80`, the merge of Hybrid
Memory PR #12. This is a usability pass on the existing local application before
Phase 3. It does not mark the Phase 2 manual acceptance gate complete.

## 1. Product intent

The user wants a simple, pleasant terminal application inspired by the clarity
of Grok Bot: a familiar list beside a useful workspace, friendly agent names,
obvious actions, and consistent keys. The current screen exposes storage IDs,
several competing panes, and different navigation rules across features.

The approved base is Option B: numbered navigation across the top, a list on the
left, and the current workspace on the right. Color and short agent initials
add personality. The design applies to Agents, Skills, Memory, and the rest of
the application, not only to the Memory entry point.

Approved decisions:

- Tab means next panel or section; Shift+Tab reverses that order.
- WASD is the advertised directional navigation. Arrow keys remain quiet
  equivalents in navigation mode.
- Number keys select the main destinations; list rows do not reuse them.
- Visible NAV and TYPE states distinguish navigation from ordinary typing.
- UUIDs, hashes, and technical bookkeeping are hidden in everyday screens.
- Names, short descriptions, counts, understandable statuses, and useful
  version labels replace the raw metadata.

## 2. Reference screens and corrections

Approved appearance references:

- [Agents](assets/two-pane/agents-concept.png)
- [Memory](assets/two-pane/memory-concept.png)
- [Future chat](assets/two-pane/chat-concept.png)

These are generated design illustrations, not screenshots of implemented
features. This written specification resolves their inconsistencies:

- Tab always advances focus between sections. It never means next note or
  next chat. W/S selects the next item in a list.
- A selected row and an active control must have different visual treatments.
  Only the section receiving keyboard input gets the strong focus treatment.
- A missing model connection must not have a green ready dot. Existing
  readiness is authoritative.
- Counts come from actual data. Example notes and conversations are mock data.
- Chat and connection setup are Phase 3 features. Their mock content does not
  become a simulated working feature in this usability pass.
- The production rendering uses a terminal cell grid, text, and box characters.
  The images' rounded badges, artwork, and varying font sizes express hierarchy;
  they do not require embedded images, a special font, or a graphics protocol.

## 3. Global navigation

The approved numbered locations remain stable as later phases arrive:

| Key | Label | Behavior in this usability pass |
| --- | --- | --- |
| 1 | Home | Friendly local status and shortcuts to existing workspaces. |
| 2 | Chat | Clear “Coming in Phase 3” page; no live input or sample replies. |
| 3 | Agents | Agent list and selected agent workspace. |
| 4 | Skills | Reusable skill library and selected skill workspace. |
| 5 | Connections | Clear “Coming in Phase 3” page; no credential entry. |
| 6 | Activity | Recent readable activity from the existing bounded audit data. |
| 7 | Setup | Existing setup status and links to current configuration features. |
| 8 | Audit | Inspectable event history, with technical details available on demand. |
| 9 | Help | Keyboard help and short task walkthroughs. |

Home is the friendly successor to Overview. Activity and Audit use the same
existing event source: Activity presents concise summaries; Audit provides
deliberate inspection. They do not introduce new persistence or event types.
When a human name is unavailable, use a neutral label such as “Agent” rather
than fabricating one or displaying a raw ID.

Selecting a destination preserves the source area's selection, scroll, nested
page, draft, and pending review. Returning restores it. A delayed response may
update the correct area's data but cannot steal focus or replace another
selected object's content.

The old bare `a` Agents and `s` Skills shortcuts are replaced by `3` and `4`:
those letters now belong to WASD. Existing slash-command grammar remains
available, and Help explains the new shortcuts.
Command outcomes retain their destinations: `/status` opens Home,
`/setup status` opens Setup, `/audit tail` opens Audit, and help opens Help.

## 4. Focus and keyboard behavior

Tab cycles through the screen's enabled logical regions in a fixed order: main navigation,
left list, right workspace, contextual actions, then back to main navigation.
Missing or empty action regions are skipped. If a control group is already the
workspace's whole contents, it appears once in the cycle, not twice. Each region
remembers its current selection. The main navigation remains reachable through
Tab as well as number keys.
In compact mode the same logical order applies: focusing a section reveals it
and hides the previous pane when necessary. An inspector, absent control, or
unrelated background workflow is never part of the focus order.

Inside an editor, the visible fields are its ordered sections: Tab advances to
the next field, then review/actions, then the surrounding screen. Moving focus
retains the exact draft, including invalid text, with any validation error shown
beside its field. Tab does not save, approve, discard, or select another record.

| Key | NAV behavior | TYPE behavior |
| --- | --- | --- |
| Tab / Shift+Tab | Next / previous section, revealing it in compact mode. | Retain text, leave typing, and move to the next / previous section or field. |
| W / S | Previous / next row; scroll a document; move vertically in a card grid. | Literal characters. |
| A / D | Move left / right within a horizontal control group. Else A goes back one layer and D opens the selected item. | Literal characters. |
| Enter | Open an item, enter a text field, or activate the visibly selected action. | Accept the current single-line field; add a newline in a multiline field. |
| 1–9 | Jump to the corresponding main area, retaining pending work. | Literal digits. |
| Esc | Close one layer or return to the parent; explicit discard is a separate action. | Keep the current draft text and return to NAV on that field. |
| / | Open the optional command field, if no pending workflow owns it. | Literal slash. |
| N / E / H | Contextual New / Edit / History shortcuts when shown. | Literal characters. |
| Arrow keys | Equivalent directional navigation to WASD. | Conventional cursor movement; never switch areas. |

For horizontal choices A/D stops at the ends; hitting A again at the first
choice does not unexpectedly navigate back. Esc remains the consistent way
out. D can open read-only detail or advance a review, but cannot perform the
final durable action. That requires a deliberate Enter press on the labeled
confirmation action. Repeated key events cannot cause additional writes.

In NAV, shortcuts accept unmodified lowercase or shifted uppercase letters.
Ctrl/Alt/Meta combinations do not trigger navigation accidentally. Pasted text
is accepted only by TYPE; it never executes a shortcut or command. PageUp,
PageDown, Home, and End remain available for long lists and documents.

TYPE is entered deliberately with Enter on a text field. N/E opens the relevant
workflow: a text field can start in TYPE, while template, source, and reference
pickers stay in NAV so WASD works. A text field in TYPE shows a cursor and a
visible TYPE label. Changing focus with
Tab returns to NAV, so letters cannot silently replace a different field.
Literal text remains literal through submission: a leading colon such as
`:back` in a profile description must not be interpreted as an editor command.
TUI actions are separate controls. Existing fallback colon controls remain
available only through their existing command-mode editor protocol.
Fields are displayed in the workspace; the global command line appears only
when requested. `/quit` and Ctrl+C retain their existing shutdown meanings;
bare q remains inert.
Enter submits a command only in that deliberately opened command field. A
slash command typed into a profile, skill, or note field remains field content.

The footer describes only the actions that work in the current context. It
always exposes Tab, the applicable WASD movement, Enter, and Esc. Help contains
the complete reference. Future chat will use Enter to send as explicitly
shown beside its composer, but chat interaction is outside this implementation.

## 5. Screen behavior

### Agents

The left pane shows an agent's initials, name, short specialty, and truthful
status. Moving the selection immediately updates the right-hand identity and
summary from that row. While full data loads, show “Loading Bear Researcher…”;
never leave another agent's details under the newly selected name.

The right pane starts with the agent's name and purpose, then four obvious
choices: Profile, Memory, Skills, History. A/D moves among choices, or W/S in
their narrow vertical form. Tab moves between the list and workspace without
requiring a hidden “open detail first” step.

Profile exposes name, role, specialty, personality, instructions, tags, and
binding status with a visible Edit action. New opens the existing guided
template flow. Profile history displays `Version 3 · Current` and prior version
labels, human dates, and readable field changes. A historical revision is
clearly labeled; it never quietly becomes the active editing target.

Readiness labels map to the existing domain state: Unbound becomes “Needs
connection”; unavailable bindings become “Connection unavailable”; Ready becomes
“Bindings configured” when the required catalog bindings are available. Additional
role-specific requirements use readable labels such as “Needs engineering
connection.” Local profile, skill, and memory management remain usable without
a model connection. “App available” describes local runtime health separately.
These labels do not promise that model calls or chat are supported, or grant
execution permission.

### Memory

A breadcrumb names the agent and Memory. Visible category controls expose
Notes, Suggestions, and Summaries, with counts when already available.
Switching categories uses an ordinary focused control group and retains the
two-pane layout.

The left list contains note names, suggestion labels, or summary labels. Lists
and automatic adjacent previews remain metadata-only. Choosing a row selects
it; Enter/D deliberately opens its full content in the right workspace. An
unopened note shows its title and an “Enter to read” hint. If selection changes,
the previous note's body is cleared immediately. Tab alone is not a content-read
action.
Preserve bounded pages and displayed/omitted counts. The UI must not present a
bounded slice as the complete collection or add unbounded database reads.

Notes show readable values and tags. N creates a note; Edit, History, and Delete
are visible actions. History lists exact immutable versions with friendly
version labels and dates; deleting a note does not imply erasing its history.

Suggestions preserve pending, approved, rejected, and stale outcomes, shown as
“Needs review,” “Accepted,” “Declined,” and “Out of date.” Deliberate detail shows
the author, affected note, rationale, proposed change, and accept/decline
actions. Summaries retain their source links and “Check the sources”
qualification, with read-only content. Source rows use a human label plus an
event number for disambiguation; raw IDs remain technical details.

The editor labels the fields Name, Note, and Tags. Note is a genuine multiline
field. Domain validation, size limits, tag normalization, and existing
review semantics remain authoritative. A short persistent local-plaintext
notice stays legible. Review explains that earlier versions may remain in
history; it does not promise secure deletion or encryption.

### Skills

The global library lists skill names, Built-in/Custom, and useful version labels.
Detail shows what the skill teaches and its content, with visible create,
new-version, history, and assignment actions. Describe skills as guidance;
avoid suggesting that assigning one grants executable capabilities.

From an agent, Skills shows that agent's exact assigned versions, whether an
update is available, and explicit assign, update, or remove actions. Opening
global skill detail preserves the agent origin so Esc returns to the same
assignment context. History supports deliberate exact-version selection;
updates and assignments cannot silently substitute the latest version.
Skill creation supports the existing blank and starter paths. Assignment
supports deliberate historical reassignment. An older target is labeled “Reassign”; an
already-assigned version is a no-op. A missing catalog cannot imply an upgrade
is available.

### Home, Setup, Activity, Audit, and Help

Home presents the current local status and useful next actions. It omits
installation IDs, session IDs, locks, and schema details. Setup describes its
actual existing state and links to the supported profile and skill tools.
Future onboarding is labeled as future work rather than made interactive.

Activity uses readable verb phrases, object names when authoritatively
available, and human dates. Audit adds a selectable event list and explanatory
detail. An optional “Technical details” action exposes IDs and digests for the
selected object or event within the right workspace. Closing that view restores
the readable screen; it does not add a permanent inspector.

Help provides short walkthroughs such as “Open an agent's memory” and “Edit a
skill,” alongside the consistent key reference and migration from a/s to 3/4.
It explains that WASD and number shortcuts operate in NAV, while all ordinary
text is literal in TYPE.

## 6. Reviews, errors, and continuity

Review and confirmation remain distinct visible steps. A review names the
target, shows the changes, and offers Continue/Back. Confirmation uses friendly
action text such as “Save changes to Investment thesis for Bear Researcher.”
No one must read or type a UUID or digest in the TUI. Exact review tokens,
version references, expected state, and authenticated commands remain internal.
Fallback command mode keeps its existing exact protocol and grammar.

A field change invalidates an earlier preview. Stale or mismatched results
cannot replace a current review or authorize an action. A stale confirmation
shows a readable instruction to review the current version again. Cancel,
shutdown, and terminal failures retain the existing review cleanup guarantees.

Leaving a field or switching areas preserves the draft. Existing rules that
prevent changing a reviewed object's identity while its review is pending
remain in force, with a readable explanation. A user can deliberately discard
a draft via a labeled action. Runtime responses update only their bound target;
they cannot replace the selection or focus made after the request started.
This preserves the existing single protected memory workflow; it does not add
simultaneous independent memory drafts for every agent. Retryable failures may
retain the exact review; invalid or terminal reviews require a fresh preview
while preserving editable input where the existing workflow allows it.

Success uses a short message, for example “Note saved,” and keeps the relevant
item selected. Recoverable errors stay beside the affected field or action and
preserve editable input. Technical error codes are available in details; they
do not replace the human explanation. Terminal-unsafe text is still escaped.

## 7. Appearance and terminal sizing

Use dark charcoal surfaces, clear near-white text, muted secondary copy, and
cyan for the active focus. Give agents stable colored initials derived from
their internal identity. Ordinary reordering does not change an agent's color.
Names remain the primary identity; duplicate initials are harmless. Readiness
uses explicit words plus status markers, separately from agent color.

Use modest padding, a single divider between the panes, short descriptions,
and small terminal-safe monograms. Optional text artwork appears only when
space remains after all meaningful content. There are no mandatory animations,
flashing states, special glyph fonts, or inaccessible color-only controls.
NO_COLOR retains focus through reverse video/bold and a visible marker.

- At 100 columns and above, show both panes. The list uses about 28% of width,
  clamped to 24–36 cells; the workspace gets the remainder. Wide screens do not
  acquire a third pane. Agent actions use a horizontal row where they fit and
  otherwise a vertical menu.
- From 60–99 columns, retain both logical panes but display one at a time.
  Tab reveals the next focused section, preserving selection and draft state.
  Breadcrumbs explain the current location. This is the compact form of the
  same model, not an additional navigation scheme.
- Main navigation wraps whole numbered labels across rows at word boundaries.
  Numbers keep their destinations at every width. The product title yields
  space before navigation labels are truncated.
- The supported minimum remains 60×18. At short heights, artwork and secondary
  prose yield first. Identity, mode, field errors, and confirmation actions stay
  visible; long content scrolls within its region. Below minimum size, preserve
  state and show a resize message while keeping safe exit available.

## 8. Implementation boundaries and delivery order

The existing domain, SQLite storage, migrations, event authority, immutable
history, policy, and command receipts remain the foundation. IDs stay as typed
identities internally. The redesign changes presentation and keyboard routing;
it does not turn displayed labels into persistence keys.

Primary seams in the current tree:

- `src/ui/tui/model.rs`: explicit destinations, focus regions, input ownership,
  saved per-area state, and selected object identity.
- `src/ui/tui/controller.rs`: one keyboard routing policy before feature actions.
- `src/ui/tui/layout.rs`, `render.rs`, and `theme.rs`: shared two-pane geometry,
  header, footer, readable metadata formatting, and focus treatment.
- `src/ui/tui/views/*`: friendly feature lists, details, and action groups.
- `src/ui/tui/host.rs`: retain authoritative review/data operations and restore
  the correct origin after results. Factor only touched presentation plumbing
  where necessary; avoid a separate rewrite of the runtime.
- `src/ui/*_editor.rs`: keep draft validation and deliberate commit flows while
  exposing conventional fields and explicit input mode in the TUI.

The first implementation slice, the shared navigation/focus shell plus the
complete Agents path, is implemented. Full Memory and Skills usability passes,
then the remaining status/history/help views, are future slices. The first
slice has automated and rendered-preview evidence, but its manual `make dev`
acceptance gate remains unperformed. Chat and provider execution remain Phase 3
work.

Update README, the relevant manual guides, and navigation/render assertions
when implementation lands. Historical phase specifications stay historical.
Tests that insist on six destinations, a/s navigation, a permanent inspector,
or routine raw IDs must follow the approved new UI contract. Tests of typed
identity, isolation, exact reviews, replay, and durable state must continue to
verify those guarantees.

## 9. Acceptance for the implementation

1. Start through normal `make dev`. Press 3, choose an agent with W/S, Tab into
   its workspace, select Memory with A/D, and Enter to open it. No invisible
   focus prerequisite or ID lookup is needed.
2. Tab and Shift+Tab cycle the screen's enabled sections, revealing the
   destination in compact mode and skipping absent controls. They never
   choose another agent, note, history version, or chat by themselves.
3. Type text containing `wasd123456789/n` and leading `:back` in each applicable
   text field. It stays text. Esc retains it, Tab changes focus predictably,
   and a multiline field accepts
   line breaks without submitting a durable edit.
4. Create and edit a profile, manage exact skill assignments, create/edit/delete
   a note, and inspect history using the visible actions and friendly labels.
5. Exercise acceptance and rejection with synthetic proposals using existing
   isolated automated fixtures. Preserve exact reviewed targets, rejection of
   stale state, replay, cleanup, and namespace isolation.
6. Show no automatic memory prose preview or stale detail belonging to another
   selected object. Hide routine technical metadata in all normal screens,
   including errors and confirmations; deliberate technical detail still works.
7. Rapidly change areas and selection while a load is pending. Drafts, focus,
   selection, and target-bound review state remain correct on return.
8. Check 60×18, 80×24, 100×24, 120×30, and 160×40 terminal sizes, plus resize
   during editing/review and NO_COLOR. Critical content is legible and all
   actions remain reachable without a third pane.
9. Existing direct-provider and room functionality is not implied by the UI:
   unavailable future destinations explain what is coming and retain navigation.

Verification should emphasize behavior and representative render snapshots,
with focused controller, host, editor, and integration tests. A documentation
commit alone requires link, scope, and whitespace checks; it does not require
rerunning the product's full test suite.
