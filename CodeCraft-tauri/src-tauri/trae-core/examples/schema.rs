fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&codecraft_trae::protocol::schemas()).unwrap()
    );
}
