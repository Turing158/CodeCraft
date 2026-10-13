pub mod capture;
pub mod mcp;
pub mod compatibility {
    pub use codecraft_cursor::compatibility::*;
}
pub mod files {
    pub use codecraft_cursor::files::*;
}
pub mod history {
    pub use codecraft_cursor::history::*;
}
pub mod install {
    pub use codecraft_cursor::install::*;
}
pub mod projects {
    pub use codecraft_cursor::projects::*;
}
pub mod protocol {
    pub use codecraft_cursor::protocol::*;
}
pub mod schema {
    pub use codecraft_cursor::schema::*;
}
pub mod store {
    pub use codecraft_cursor::store::*;
}
pub mod tickets {
    pub use codecraft_cursor::tickets::*;
}
pub mod transport {
    pub use codecraft_cursor::transport::*;
}
pub mod workflow {
    pub use codecraft_cursor::workflow::*;
}
pub fn limit_cpu() -> Result<(), String> {
    codecraft_cursor::process::limit_cpu().map_err(str::to_owned)
}
