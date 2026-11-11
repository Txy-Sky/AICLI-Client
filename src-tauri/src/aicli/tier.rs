//! Which clients AICLI can take over, and how.
//!
//! Tier 1 clients have a route mode and go through the mode controller. Tier 2
//! clients (J stage) only get one `aicli` provider written by their own config
//! writer, pointing at the generic `/app/<client>/...` entry.
//!
//! This list is what the backend is able to do. The UI offers a takeover button
//! for a Tier 2 client only after its real client passed the J acceptance
//! (`AICLI_TAKEOVER_APPS` in the frontend); until then it stays "coming soon".
use crate::app_config::AppType;

/// Tier 2 clients whose takeover and restore are implemented.
pub const TIER2_SUPPORTED: [AppType; 1] = [AppType::OpenCode];

pub fn is_tier2(app: &AppType) -> bool {
    TIER2_SUPPORTED.contains(app)
}

/// The protocol the client's `aicli` provider speaks to `/app/<client>/...`.
pub(crate) fn tier2_protocol(_app: &AppType) -> &'static str {
    // OpenCode: `@ai-sdk/openai-compatible`, i.e. Chat Completions.
    "openai"
}
