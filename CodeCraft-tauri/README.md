# CodeCraft

CodeCraft is a compact Windows desktop toolbar built with Rust, Tauri 2, and the system WebView2 runtime.

## Development

Prerequisites:

- Node.js and npm
- Rust stable with the `x86_64-pc-windows-msvc` target
- Visual Studio C++ Build Tools and the Windows SDK

Commands:

```powershell
npm install
npm test
npm run build
npm run tauri dev
```

Create the Windows installer with:

```powershell
npm run tauri build
```

Create a portable single executable (without an installer) with:

```powershell
npm run package:single-exe
```

The result is written to `artifacts\CodeCraft.exe`. The executable embeds the
frontend and Rust application resources; Windows still needs the WebView2
Runtime, which is already included with current Windows 10/11 installations.

## Trae CN integration

The current integration targets Trae CN `3.3.102` on Windows. Six user-level Hook events synchronize sessions, collected messages, and tool activity. Ordinary tool requests can be allowed once, denied, or returned to Trae; native questions and Plan/Spec reviews remain read-only and must be handled in Trae. No additional MCP service is required.

- See the [Trae integration guide](protocol/trae/README.md) (Chinese) for installation, compatibility limits, and verification scope.
- See the [verification scripts](scripts/README.md) (Chinese) for the isolated desktop/LAN previews and regression checks.

Run the Trae regression checks from this directory:

```powershell
.\scripts\check-trae.ps1
```

Add `-Full` to include the main application, frontend, and build checks. The script sets `CARGO_BUILD_JOBS=2` and limits its process and child processes to two logical CPU cores.
