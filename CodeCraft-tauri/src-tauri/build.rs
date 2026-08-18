use std::{fs, path::Path};

/// The LAN console page is embedded with include_str!, so cargo must find the
/// file even before the web build has produced it. A placeholder keeps a fresh
/// checkout compiling instead of failing on a missing include.
fn ensure_lan_console_placeholder() {
    let path = Path::new("assets/lan/index.html");
    println!("cargo:rerun-if-changed=assets/lan/index.html");
    if path.exists() {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(
        path,
        "<!doctype html>\n<html lang=\"zh-CN\">\n<head><meta charset=\"utf-8\" />\
<title>CodeCraft 局域网控制台</title></head>\n<body>\
<p>局域网控制台尚未构建，请先运行 npm run build。</p>\n</body>\n</html>\n",
    );
}

fn main() {
    ensure_lan_console_placeholder();
    tauri_build::build()
}
