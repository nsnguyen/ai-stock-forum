# Phase 0B Adaptive Cockpit Acceptance Guide

This guide documents the release acceptance procedure for the Phase 0B
read-only terminal UI. Normal use may launch the application with persistent
state, but destructive acceptance experiments must use the isolated-state
procedures below. Run live-TTY checks only after task review.

## Launch modes and fallback

Use these launch examples exactly:

```text
cargo run --locked
cargo run --locked -- --command-mode
printf '/status\n/quit\n' | cargo run --quiet --locked
```

`cargo run --locked` selects the alternate-screen Adaptive Cockpit only when
both stdin and stdout are terminals. `--command-mode` always selects the
existing line host. Redirection of either stdin or stdout also selects that
line host automatically, preserving its existing text rendering and command
protocol. The piped example is therefore expected to return the `/status`
output, then process `/quit`, without a full-screen UI.

## Isolated state for acceptance experiments

`AppPaths::discover` uses `BaseDirs::data_dir()` and appends
`ai-stock-forum`. The application therefore uses the standard macOS data
directory derived from `HOME`, Linux `XDG_DATA_HOME` when set (otherwise the
standard `HOME` fallback), and Windows `APPDATA`. Build before replacing those
environment values so Cargo and Rustup continue to use their normal locations.

On macOS, build first, then run the already-built binary with a temporary
`HOME`; the application state will be under the temporary home directory:

```sh
cargo build --locked
state_root="$(mktemp -d)"
mkdir -p "$state_root/home" "$state_root/xdg-data"
HOME="$state_root/home" XDG_DATA_HOME="$state_root/xdg-data" \
  target/debug/ai-stock-forum
```

On Linux, build first, then use temporary `HOME` and `XDG_DATA_HOME`; the
application state will be under `$state_root/xdg-data/ai-stock-forum`:

```sh
cargo build --locked
state_root="$(mktemp -d)"
mkdir -p "$state_root/home" "$state_root/xdg-data"
HOME="$state_root/home" XDG_DATA_HOME="$state_root/xdg-data" \
  target/debug/ai-stock-forum
```

In PowerShell on Windows, build first, then use a generated temporary home and
`APPDATA`; the application state will be under `$testAppData\ai-stock-forum`:

```powershell
cargo build --locked
$stateRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("ai-stock-forum-phase-0b-" + [guid]::NewGuid().ToString("N"))
$testHome = Join-Path $stateRoot "home"
$testAppData = Join-Path $stateRoot "appdata"
New-Item -ItemType Directory -Path $testHome, $testAppData | Out-Null
$previousHome = $env:HOME
$previousAppData = $env:APPDATA
try {
    $env:HOME = $testHome
    $env:APPDATA = $testAppData
    & .\target\debug\ai-stock-forum.exe
} finally {
    if ($null -eq $previousHome) { Remove-Item Env:HOME -ErrorAction SilentlyContinue } else { $env:HOME = $previousHome }
    if ($null -eq $previousAppData) { Remove-Item Env:APPDATA -ErrorAction SilentlyContinue } else { $env:APPDATA = $previousAppData }
}
```

After the application exits, inspect the generated temporary directory before
removing it. Use an interactive deletion only for the `state_root` created by
the matching procedure, never for a standard application directory:

```sh
rm -ri -- "$state_root"
```

```powershell
Remove-Item -LiteralPath $stateRoot -Recurse -Confirm
```

## Interactive controls

The cockpit has native Overview, Setup, Audit, and Help views; it must never
emit a transcript while navigating them.

| Control | Expected result |
| --- | --- |
| `1`-`4`, `?` | Select Overview, Setup, Audit, or Help. |
| `Tab`, `Shift+Tab` | Cycle visible focus forward or backward. |
| `Up`, `Down`, `Left`, `Right`, `PageUp`, `PageDown`, `Home`, `End` | Navigate the focused region, including Audit selection. |
| `i` | Open or focus the inspector. |
| `Esc` | Dismiss the inspector or message; in command focus, clear and leave command entry. |
| `/` | Move to command focus with a leading slash. |
| Text, `Enter`, `Backspace`, `Delete`, editor arrows, editor `Home`/`End`, editor `Up`/`Down` | Edit, submit, and recall the bounded in-memory command history. |
| `q` | Request an auditable clean user shutdown when outside command entry. |
| `Ctrl+C` | Request clean interrupted shutdown from any focus. |

Only one application command can be pending. While one is pending, a second
submission is refused locally and cannot race the existing runtime command.

## Layout, color, and restoration

The minimum usable terminal is `60x18`. Resizing below either minimum displays
the TooSmall guidance screen; non-shutdown controls are ignored there. At or
above the minimum, the layout is Narrow until both `80x24` are met, Medium
until both `120x30` are met, and Wide thereafter.

Set `NO_COLOR=1` for the no-color check. The cockpit must use no foreground or
background colors, while focus remains distinguishable through non-color
styling. Mouse capture remains disabled.

For `q`, `Ctrl+C`, a forced UI error, and the panic seam, verify that raw mode,
cursor state, and alternate-screen state are restored before control returns to
the shell. Normal user quit exits successfully and is auditable. Error and
panic paths print at most one safe summary after restoration; the summary must
not expose a command payload, a local path, a panic payload, or a raw internal
error.

With a first process holding the same isolated state directory, start a second
process. The existing single-instance protection must reject the second process
safely without corrupting the first session.

## Acceptance matrix

| Scenario | Expected result |
|---|---|
| Interactive launch | Alternate-screen cockpit appears; shell contents are not overwritten. |
| `--command-mode` | Existing line protocol and text rendering remain unchanged. |
| Redirected stdin | Command mode is selected automatically. |
| Redirected stdout | Command mode is selected automatically. |
| Widths 59, 60, 79, 80, 119, 120 | TooSmall, Narrow, Narrow, Medium, Medium, Wide when the matching height threshold is met. |
| Heights 17, 18, 23, 24, 29, 30 | TooSmall below 18; width-dependent Narrow/Medium/Wide above it. |
| Keys `1`-`4`, Tab, arrows, paging, `/`, Esc, `i`, `?` | Focus and native views update without transcript output. |
| `/help`, `/status`, `/setup status`, `/audit tail`, `/audit tail N`, invalid input | Existing application command semantics and audit behavior are preserved. Bare `/audit` is rejected as malformed input. |
| `q` | Auditable clean shutdown, terminal restored, success exit. |
| Ctrl+C | Clean external-signal shutdown and terminal restoration. |
| Forced UI error/panic seam | One safe line after restoration; no payload or path leakage. |
| `NO_COLOR=1` | No foreground/background colors; focus remains visibly distinct. |
| Second process | Existing single-instance protection rejects it safely. |
| macOS, Linux, Windows | Keyboard input, resizing, shutdown, and restoration meet the same contract. |

## Verification record and release gates

Automated and static coverage is available across platform paths. Live manual
verification is host-specific: Task 13 did not run a live interactive binary,
and therefore does not claim macOS, Linux, or Windows live execution. Record
the host, terminal, isolated-state arrangement, and each completed matrix row
when the controller performs the live-TTY acceptance pass. In particular, do
not claim Windows or Linux live verification unless it actually occurred.

Run these release gates after documentation and any focused gate correction:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --locked
```
