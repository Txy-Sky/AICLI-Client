pub mod account;
pub mod alias;
pub mod cache;
pub mod catalog;
pub mod cooldown;
pub mod errors;
pub mod lines;
pub mod local_auth;
pub mod newapi;
pub mod picker;
pub mod privacy;
pub mod profiles;
pub mod resolver;
pub mod runtime;
pub(crate) mod schema;
pub mod secrets;
pub mod seed;
pub mod takeover;
pub mod tier;
pub mod tokens;
pub mod usage;

#[cfg(test)]
pub(crate) mod testing;

/// Explicit test-only surface for the real-proxy integration acceptance.
#[cfg(feature = "test-hooks")]
pub mod routing_test_support {
    pub use crate::claude_desktop_config::get_or_create_gateway_token;
    pub use crate::proxy::{
        handler_context::{RequestContext, RouteInput},
        server::ProxyServer,
        types::ProxyConfig,
    };

    pub fn connection(db: &crate::Database) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
        db.conn.lock().unwrap()
    }
}
