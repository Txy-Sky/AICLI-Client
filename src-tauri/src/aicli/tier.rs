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
///
/// Antigravity CLI (`agy`) is not here: it reads its endpoint and key only from
/// the process environment, so no config file can point it at the local proxy,
/// and AICLI never rewrites its real `settings.json` (J-10).
pub const TIER2_SUPPORTED: [AppType; 8] = [
    AppType::OpenCode,
    AppType::OpenClaw,
    AppType::Hermes,
    AppType::Pi,
    AppType::Mcode,
    AppType::Kimi,
    AppType::Dsh,
    AppType::Omp,
];

pub fn is_tier2(app: &AppType) -> bool {
    TIER2_SUPPORTED.contains(app)
}

/// The protocol the client's `aicli` provider speaks to `/app/<client>/...`.
pub(crate) fn tier2_protocol(_app: &AppType) -> &'static str {
    // Every Tier 2 client so far gets an OpenAI-compatible (Chat Completions) provider.
    "openai"
}
