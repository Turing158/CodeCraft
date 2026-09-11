# Kimi Code Hook: Read-Only Support

## Verified Scope

Kimi Code CLI 0.41.0 on Windows was exercised on 2026-09-07 and 2026-09-08 using an isolated
KIMI_CODE_HOME, a local deterministic model, and the compiled CodeCraft hook.
Each capture within a run had the same Kimi ancestor PID and creation time.
Tool output, question options, and plan review content survived capture.
No SessionEnd was emitted in these runs. Stop means an idle turn, not an
ended session.

The following fixtures preserve captured event order and structure, with
session IDs, paths, PIDs and timestamps normalized:

| Fixture | Verified behavior |
| --- | --- |
| `0.41.0/read-tool-stop.json` | Prompt array, Read input/output, turn completion |
| `0.41.0/question-failure-stop.json` | AskUserQuestion text/options, late TurnStarted, PostToolUseFailure |
| `0.41.0/plan-permission-stop.json` | EnterPlanMode, plan Write, ExitPlanMode, PermissionRequest/PermissionResult with `display.plan`, PostToolUse after Stop |

The question probe uses `kimi -p`, where interactive questions cannot be
answered and the native tool reports failure. It verifies observation and
cleanup, not a successful interactive answer. The plan probe uses an isolated
ACP fixture client because `kimi -p` automatically approves plans; that client
selects `allow_once` only for the local fixture. CodeCraft itself remains
Hook-only. Live terminal focus still requires the manual check below.

Hook processes can deliver events out of order. The plan capture contains
PermissionResult before PermissionRequest, and PostToolUse after Stop.
Resolved calls retain their closed status when later notifications enrich
their content. Stop, StopFailure and Interrupt close the turn until a new
prompt arrives. Unverified events retain synthetic tests or documented mappings.

Official contract: <https://moonshotai.github.io/kimi-code/en/customization/hooks>

## Capabilities

| Capability | Status |
| --- | --- |
| Observe sessions, prompts and tools | Enabled |
| Display question and plan observations | Read-only |
| Approve tools, answer questions, approve plans | Disabled |
| Stream assistant output | Unavailable in the verified Hook sequence |
| LAN writes for Kimi | No Kimi decision endpoint |
| Navigate to original terminal | Best effort; unavailable without a verified target |
| Desktop Kimi client or other CLI versions | Not verified |

The CodeCraft desktop routes active tool, question, and plan observations to
its existing permission, question, and plan views. These views are read-only:
the only Kimi action opens the originating terminal. Completed, interrupted,
or ended observations close the review; returning to the session list dismisses
that observation until it is selected manually or a new one arrives.

Navigation validates the captured Kimi process creation time and the window
owner's process creation time before focusing it. Classic console windows can
be focused directly. Windows Terminal and supported IDE hosts are treated as
shared windows, where selecting the terminal tab may still be necessary.
Missing or ambiguous targets produce a message instead of focusing an
unrelated window.

Browser-only previews are available at `?previewKimi=permission`,
`?previewKimi=question`, and `?previewKimi=plan`. They use sample observations;
native terminal navigation requires the desktop app and a real Kimi Hook.

The callback emits no stdout and returns success even when capture fails.
This preserves native Kimi processing; capture errors do not mean approval or
denial. The verified contract has no stable asynchronous decision ID for the
three decision types in the implementation plan. Full bidirectional support
must remain gated on independent protocol evidence.

## Identity and Privacy

Windows capture walks the local process tree through the command shell to a
Kimi ancestor. Payload-supplied process and window identifiers are discarded.
Session identity includes native ID, cwd, PID and creation time. Missing cwd
can match only one compatible stored session. Orphans, ambiguous matches,
older events and events after SessionEnd are quarantined. If ancestor lookup
fails, observation identity is weaker and navigation is unavailable.

Input is limited to one JSON document and 1 MiB. Captures are bounded and
written via temporary files. Sensitive keys and common credentials in text
are removed or masked before persistence. Plan files must resolve under the
session directory or configured plan roots, and their text is also redacted.
Redaction is best effort; arbitrary secrets in prose cannot be recognized
reliably. LAN omits local terminal binding and process instance metadata.

## Configuration

Install and uninstall operate on the user-level `.kimi-code/config.toml`.
TOML values and other handlers are preserved; formatting/comments are not.
Configuration writes use an exclusive lock, byte comparison, a timestamped
`.bak`, and a flushed same-directory temporary file. Reinstallation with an
identical configuration is a no-op. Uninstall removes handlers carrying the
CodeCraft Kimi command marker and retains backups.

Automatic backup restoration, per-install generation validation, verified
Desktop compatibility, and the plan's bidirectional decision service are
not implemented. This integration must be described as read-only observation,
not full completion of the v2 adaptation plan.

## Reproduce

Run from CodeCraft-tauri in PowerShell:

```powershell
(Get-Process -Id $PID).ProcessorAffinity = 3
$env:CARGO_BUILD_JOBS = '2'
$env:RUST_TEST_THREADS = '2'
cargo build --manifest-path src-tauri/Cargo.toml --bin codecraft-tauri
node scripts/kimi-protocol-probe.mjs 'C:/Users/USER/.kimi-code/bin/kimi.exe' 'src-tauri/target/debug/codecraft-tauri.exe'
node scripts/kimi-protocol-probe.mjs 'C:/Users/USER/.kimi-code/bin/kimi.exe' 'src-tauri/target/debug/codecraft-tauri.exe' question
node scripts/kimi-protocol-probe.mjs 'C:/Users/USER/.kimi-code/bin/kimi.exe' 'src-tauri/target/debug/codecraft-tauri.exe' plan
```

Probe files remain under src-tauri/target/kimi-probe. The probe changes no
user-level Kimi configuration and makes model requests only to localhost.

For the three desktop views, install or repair the Kimi Hook from the current
CodeCraft executable, enable approval auto-expand, then restart an interactive
`kimi` session. Request a tool that needs native permission, AskUserQuestion,
and a plan ending with ExitPlanMode. Confirm the corresponding read-only view,
use its terminal button, and finish the request in Kimi. The view should close
when its result arrives. Repeat with two sessions to check the selected target;
shared terminal hosts may require selecting the correct tab. See the
[Chinese walkthrough](../../../docs/USER_GUIDE.md#kimi-code-三个只读页面的手动测试).
