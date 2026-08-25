# OpenCode P0 runtime probe

This directory contains a temporary, non-product probe for the selected OpenCode integration plan. It runs OpenCode in an isolated `%TEMP%` data/config directory and starts only `opencode serve`; it does not send a model prompt.

Run on Windows with OpenCode installed:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\opencode-p0\run-server-probe.ps1
```

The probe uses `/affinity 3` equivalent process affinity through `ProcessorAffinity = 0x3`, and writes its temporary output below `%TEMP%\codecraft-opencode-p0-server-runtime`. The tracked `capability.fixture.json` is deliberately sanitized and uses placeholder paths.

## Verified result

OpenCode stable `1.18.18` and dev `0.0.0-dev-202608190652` loaded the local plugin and injected a client with `session`, `event`, `project`, `config`, and other APIs. Neither injected client exposed `question` or `permission`, so D1-A cannot implement Question/Permission list/reply/reject.

The user-authorized D1-B fallback passed against both builds. The probe created real Question and Permission pending requests through an isolated local model, then used the same OpenCode instance's localhost HTTP API to verify:

- Question list, reply, and reject
- Permission list, once, always, and reject
- The waiting session is released after every decision
- `always` permits a second matching command without another pending request

Run the full pending probe with:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\opencode-p0\run-http-pending-probe.ps1
```

The implementation channel is therefore D1-B. The product plugin must use the current process's `serverUrl` and authentication environment when calling localhost HTTP; credentials must not be written to IPC or exposed to the Rust/LAN layers.
