pub mod auth;
pub mod config;
pub mod frontend;
pub mod proxy;
pub mod router;
pub mod sse;
pub mod state;
pub mod upstream;

pub use config::{BffConfig, FrontendAssets};
pub use router::build_bff_router;
pub use state::BffState;
pub use upstream::{HttpUpstream, InProcessUpstream, Upstream};
