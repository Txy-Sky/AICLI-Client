//! Keyless, internal Provider rows backing the fixed AICLI lines.
use serde_json::json;

use crate::{
    app_config::AppType,
    database::{lock_conn, Database},
    error::AppError,
    provider::{Provider, ProviderMeta},
};

use super::lines::{line, LINES};

fn base_provider(app: &AppType, line_id: &str) -> Result<Provider, AppError> {
    let line = line(line_id).ok_or_else(|| AppError::Config("Unknown AICLI line".into()))?;
    let settings = match app {
        AppType::Claude | AppType::ClaudeDesktop => {
            json!({"env": {"ANTHROPIC_BASE_URL": line.origin()}})
        }
        AppType::Codex => json!({"auth": {}, "base_url": line.origin()}),
        AppType::GrokBuild => json!({"base_url": line.origin()}),
        AppType::Gemini => json!({"env": {"GOOGLE_GEMINI_BASE_URL": line.origin()}}),
        _ => return Err(AppError::Config("App has no AICLI route mode".into())),
    };
    let mut provider = Provider::with_id(
        format!("aicli-{}", line.id),
        format!("AICLI Client ({})", line.id),
        settings,
        None,
    );
    provider.category = Some("aicli".into());
    Ok(provider)
}

/// Internal rows remain available to routing/FKs, but not the Provider editor.
pub fn is_hidden(provider: &Provider) -> bool {
    provider.category.as_deref() == Some("aicli")
        && LINES
            .iter()
            .any(|line| provider.id == format!("aicli-{}", line.id))
}

/// Supplement missing rows on every startup, without selecting a Provider or
/// projecting any client files. A single transaction also makes concurrent
/// initializers safe; existing rows and current/failover pointers are preserved.
pub fn init(db: &Database) -> Result<usize, AppError> {
    let mut conn = lock_conn!(db.conn);
    let tx = conn
        .transaction()
        .map_err(|e| AppError::Database(e.to_string()))?;
    let mut inserted = 0;
    for app in AppType::all().filter(AppType::has_route_mode) {
        for line in &LINES {
            let provider = base_provider(&app, line.id)?;
            inserted += tx.execute(
                "INSERT OR IGNORE INTO providers
                 (id, app_type, name, settings_config, category, meta, is_current, in_failover_queue)
                 VALUES (?1, ?2, ?3, ?4, 'aicli', '{}', 0, 0)",
                rusqlite::params![provider.id, app.as_str(), provider.name, provider.settings_config.to_string()],
            ).map_err(|e| AppError::Database(e.to_string()))?;
        }
    }
    tx.commit().map_err(|e| AppError::Database(e.to_string()))?;
    Ok(inserted)
}

/// Produce a fresh keyless DTO with the adapter's existing wire selector. Never
/// save this request-specific clone: credentials are injected later in memory.
pub fn clone_for(app: &AppType, line_id: &str, wire: &str) -> Result<Provider, AppError> {
    let mut provider = base_provider(app, line_id)?;
    let format = match (app, wire) {
        (AppType::Claude | AppType::ClaudeDesktop, "anthropic") => "anthropic",
        (AppType::Claude | AppType::ClaudeDesktop, "openai") => "openai_chat",
        (AppType::Claude | AppType::ClaudeDesktop, "openai-response") => "openai_responses",
        (AppType::Claude | AppType::ClaudeDesktop, "gemini") => "gemini_native",
        (AppType::Codex | AppType::GrokBuild, "anthropic") => "anthropic",
        (AppType::Codex | AppType::GrokBuild, "openai") => "openai_chat",
        (AppType::Codex | AppType::GrokBuild, "openai-response") => "openai_responses",
        (AppType::Gemini, "gemini") => "gemini_native",
        _ => return Err(AppError::Config("Unsupported AICLI app/wire pair".into())),
    };
    provider.meta = Some(ProviderMeta {
        api_format: Some(format.into()),
        ..Default::default()
    });
    Ok(provider)
}

#[cfg(test)]
mod tests;
