// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    codecraft_tauri_lib::install_panic_hook();

    if std::env::args_os().any(|argument| argument == "--codecraft-zcode-hook") {
        if let Err(error) = codecraft_tauri_lib::capture_zcode_hook() {
            eprintln!("CodeCraft could not capture the ZCode hook: {error}");
        }
        return;
    }

    if std::env::args_os().any(|argument| argument == "--codecraft-codex-hook") {
        if let Err(error) = codecraft_tauri_lib::capture_codex_hook() {
            eprintln!("CodeCraft could not capture the Codex hook: {error}");
        }
        return;
    }

    if std::env::args_os().any(|argument| argument == "--codecraft-gemini-hook") {
        if let Err(error) = codecraft_tauri_lib::capture_gemini_hook() {
            eprintln!("CodeCraft could not capture the Gemini hook: {error}");
        }
        return;
    }

    if std::env::args_os().any(|argument| argument == "--codecraft-kimi-hook") {
        if let Err(error) = codecraft_tauri_lib::capture_kimi_hook() {
            eprintln!("CodeCraft could not capture the Kimi hook: {error}");
        }
        return;
    }

    if std::env::args_os().any(|argument| argument == "--codecraft-claude-hook") {
        if let Err(error) = codecraft_tauri_lib::capture_claude_hook() {
            eprintln!("CodeCraft could not capture the Claude Code hook: {error}");
        }
        return;
    }

    codecraft_tauri_lib::run()
}
