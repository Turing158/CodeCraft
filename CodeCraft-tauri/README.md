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
