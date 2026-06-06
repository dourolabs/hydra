//! Per-port subdomain proxy: parses `<port>-<HydraId>.proxy.<host>` host
//! labels, mints + validates per-target cookies, and dispatches to
//! `JobEngine::proxy_http`/`proxy_ws`.

pub mod cookie;
pub mod host;
pub mod state;
