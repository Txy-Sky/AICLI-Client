//! Client-facing model ids. Desktop ids are persisted in the existing AICLI cache.
use std::collections::BTreeMap;

use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::{app_config::AppType, database::Database};

const DESKTOP_KIND: &str = "desktop_alias";
const DESKTOP_PREFIX: &str = "claude-fable-aicli-";
const DESKTOP_CODES: u32 = 1_000_000;

pub fn encode(
    app: &AppType,
    public_model: &str,
    db: Option<&Database>,
) -> Result<String, AliasError> {
    match app {
        AppType::Claude => Ok(format!("aicli-claude-{public_model}")),
        AppType::ClaudeDesktop => desktop_encode(db.ok_or(AliasError::Database)?, public_model),
        _ => Ok(public_model.to_owned()),
    }
}

pub fn decode(
    app: &AppType,
    client_id: &str,
    db: Option<&Database>,
) -> Result<Option<String>, AliasError> {
    match app {
        AppType::Claude => Ok(client_id.strip_prefix("aicli-claude-").map(str::to_owned)),
        AppType::ClaudeDesktop => desktop_decode(db.ok_or(AliasError::Database)?, client_id),
        _ => Ok(None),
    }
}

pub fn default_aliases(
    app: &AppType,
    ordered_models: &[String],
    db: Option<&Database>,
) -> Result<BTreeMap<String, String>, AliasError> {
    let Some(first) = ordered_models.first() else {
        return Ok(BTreeMap::new());
    };
    let mut result = BTreeMap::new();
    for (role, needles) in [
        ("OPUS", &["opus"][..]),
        ("SONNET", &["sonnet"][..]),
        ("HAIKU", &["haiku"][..]),
        ("FABLE", &["fable"][..]),
    ] {
        let selected = ordered_models
            .iter()
            .find(|name| {
                needles
                    .iter()
                    .any(|needle| name.to_lowercase().contains(needle))
            })
            .unwrap_or(first);
        result.insert(
            format!("ANTHROPIC_DEFAULT_{role}_MODEL"),
            encode(app, selected, db)?,
        );
    }
    Ok(result)
}

#[derive(Debug, thiserror::Error)]
pub enum AliasError {
    #[error("AICLI alias storage is unavailable")]
    Database,
    #[error("Desktop alias space is exhausted")]
    Exhausted,
}

fn read_map(db: &Database) -> Result<BTreeMap<String, String>, AliasError> {
    let conn = db.conn.lock().map_err(|_| AliasError::Database)?;
    let body: Option<String> = conn
        .query_row(
            "SELECT body_json FROM aicli_cache WHERE kind=?1",
            [DESKTOP_KIND],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| AliasError::Database)?;
    body.map(|value| serde_json::from_str(&value).map_err(|_| AliasError::Database))
        .transpose()
        .map(|value| value.unwrap_or_default())
}

fn desktop_encode(db: &Database, model: &str) -> Result<String, AliasError> {
    let mut conn = db.conn.lock().map_err(|_| AliasError::Database)?;
    let tx = conn.transaction().map_err(|_| AliasError::Database)?;
    let body: Option<String> = tx
        .query_row(
            "SELECT body_json FROM aicli_cache WHERE kind=?1",
            [DESKTOP_KIND],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| AliasError::Database)?;
    let mut aliases: BTreeMap<String, String> = body
        .map(|value| serde_json::from_str(&value).map_err(|_| AliasError::Database))
        .transpose()?
        .unwrap_or_default();
    if let Some(alias) = aliases.get(model) {
        return Ok(alias.clone());
    }

    let digest = Sha256::digest(model.as_bytes());
    let mut code = u32::from_be_bytes(digest[..4].try_into().map_err(|_| AliasError::Database)?)
        % DESKTOP_CODES;
    for _ in 0..DESKTOP_CODES {
        let candidate = format!("{DESKTOP_PREFIX}{code:06}");
        if crate::mode::stack::desktop_keeps_model_id(&candidate)
            && crate::mode::stack::desktop_model_identity(&candidate) == candidate
            && !aliases.values().any(|existing| existing == &candidate)
        {
            aliases.insert(model.to_owned(), candidate.clone());
            let body = serde_json::to_string(&aliases).map_err(|_| AliasError::Database)?;
            tx.execute(
                "INSERT INTO aicli_cache(kind,fingerprint,fetched_at,body_json) VALUES(?1,NULL,?2,?3) ON CONFLICT(kind) DO UPDATE SET fetched_at=excluded.fetched_at,body_json=excluded.body_json",
                params![DESKTOP_KIND, chrono::Utc::now().timestamp(), body],
            )
            .map_err(|_| AliasError::Database)?;
            tx.commit().map_err(|_| AliasError::Database)?;
            return Ok(candidate);
        }
        code = (code + 1) % DESKTOP_CODES;
    }
    Err(AliasError::Exhausted)
}

fn desktop_decode(db: &Database, client_id: &str) -> Result<Option<String>, AliasError> {
    Ok(read_map(db)?
        .into_iter()
        .find_map(|(model, alias)| (alias == client_id).then_some(model)))
}

#[cfg(test)]
mod tests;
