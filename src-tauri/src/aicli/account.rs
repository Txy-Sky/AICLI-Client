//! Binding lifecycle. Only public account fields reach SQLite; credentials stay in SecretStore.
use std::fmt;
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;

use super::lines::line;
use super::newapi::{NewApiClient, NewApiError};
use super::secrets::{SecretError, SecretStore};
use crate::database::Database;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingState {
    NotBound,
    Bound,
    NeedsRebind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AccountInfo {
    pub user_id: i64,
    pub username: String,
    pub user_group: String,
    pub selected_line_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AccountError {
    #[error("This is an API key; paste the system access token from personal settings")]
    WrongCredentialType,
    #[error("Bind an account before sending model requests")]
    NotBound,
    #[error("Local account data is unavailable")]
    Database,
    #[error("Account credentials could not be restored; bind the account again")]
    CredentialRollback,
    #[error("Client takeover could not be restored")]
    Restore,
    #[error(transparent)]
    Api(#[from] NewApiError),
    #[error(transparent)]
    Secret(#[from] SecretError),
}

/// Mandatory lifecycle integration: C1-06 schedules refresh; C3-10 restores every takeover.
/// There is deliberately no production no-op implementation.
pub trait AccountLifecycle: Send + Sync {
    fn refresh(&self);
    fn release_all(&self) -> BoxFuture<'_, Result<(), AccountError>>;
}

trait AccountRemote: Send + Sync {
    fn verify<'a>(
        &'a self,
        access: &'a str,
        selected_line: &'a str,
    ) -> BoxFuture<'a, Result<Value, NewApiError>>;
    fn delete<'a>(
        &'a self,
        access: &'a str,
        selected_line: &'a str,
        token_id: i64,
    ) -> BoxFuture<'a, Result<(), NewApiError>>;
}

struct HttpRemote;

impl AccountRemote for HttpRemote {
    fn verify<'a>(
        &'a self,
        access: &'a str,
        selected_line: &'a str,
    ) -> BoxFuture<'a, Result<Value, NewApiError>> {
        Box::pin(async move { NewApiClient::new(access, selected_line)?.user_self().await })
    }
    fn delete<'a>(
        &'a self,
        access: &'a str,
        selected_line: &'a str,
        token_id: i64,
    ) -> BoxFuture<'a, Result<(), NewApiError>> {
        Box::pin(async move {
            NewApiClient::new(access, selected_line)?
                .delete_token(token_id)
                .await
        })
    }
}

type CredentialChange = (String, Option<String>);

struct Binding {
    state: BindingState,
    generation: u64,
}

pub struct AccountService {
    db: Arc<Database>,
    secrets: Arc<dyn SecretStore>,
    lifecycle: Arc<dyn AccountLifecycle>,
    remote: Arc<dyn AccountRemote>,
    operation: tokio::sync::Mutex<()>,
    state: Mutex<Binding>,
}

impl fmt::Debug for AccountService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AccountService")
            .field("state", &self.state())
            .finish_non_exhaustive()
    }
}

impl AccountService {
    pub fn new(
        db: Arc<Database>,
        secrets: Arc<dyn SecretStore>,
        lifecycle: Arc<dyn AccountLifecycle>,
    ) -> Result<Self, AccountError> {
        let account_exists = db
            .conn
            .lock()
            .map_err(|_| AccountError::Database)?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM aicli_account WHERE id=1)",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|_| AccountError::Database)?;
        let state = if !account_exists {
            BindingState::NotBound
        } else if secrets
            .get("access-token")?
            .is_some_and(|value| !value.is_empty())
        {
            BindingState::Bound
        } else {
            BindingState::NeedsRebind
        };
        Ok(Self {
            db,
            secrets,
            lifecycle,
            remote: Arc::new(HttpRemote),
            operation: tokio::sync::Mutex::new(()),
            state: Mutex::new(Binding {
                state,
                generation: 0,
            }),
        })
    }

    pub fn state(&self) -> BindingState {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .state
    }

    fn set_state(&self, state: BindingState) {
        let mut binding = self.state.lock().unwrap_or_else(|error| error.into_inner());
        binding.state = state;
        binding.generation = binding.generation.wrapping_add(1);
    }

    /// Capture before starting an account request; late failures cannot invalidate a new binding.
    pub fn generation(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .generation
    }

    pub fn observe_api_error(&self, generation: u64, error: NewApiError) {
        let mut binding = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if error == NewApiError::TokenInvalid
            && binding.state == BindingState::Bound
            && binding.generation == generation
        {
            binding.state = BindingState::NeedsRebind;
        }
    }

    pub fn account(&self) -> Result<Option<AccountInfo>, AccountError> {
        self.db
            .conn
            .lock()
            .map_err(|_| AccountError::Database)?
            .query_row(
                "SELECT user_id,username,user_group,selected_line_id FROM aicli_account WHERE id=1",
                [],
                |row| {
                    Ok(AccountInfo {
                        user_id: row.get(0)?,
                        username: row.get(1)?,
                        user_group: row.get(2)?,
                        selected_line_id: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|_| AccountError::Database)
    }

    pub fn require_bound(&self) -> Result<(), AccountError> {
        if self.state() == BindingState::Bound {
            Ok(())
        } else {
            Err(AccountError::NotBound)
        }
    }

    fn managed_tokens(conn: &Connection) -> Result<Vec<(i64, bool)>, AccountError> {
        let mut statement = conn
            .prepare("SELECT token_id,created_by_client FROM aicli_managed_token ORDER BY token_id")
            .map_err(|_| AccountError::Database)?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|_| AccountError::Database)?;
        rows.collect::<Result<_, _>>()
            .map_err(|_| AccountError::Database)
    }

    fn restore_credentials(&self, snapshot: &[CredentialChange]) -> Result<(), AccountError> {
        let mut failed = false;
        for (account, value) in snapshot {
            let restored = match value {
                Some(value) => self.secrets.set(account, value),
                None => self.secrets.delete(account),
            };
            failed |= restored.is_err();
        }
        if failed {
            self.set_state(BindingState::NeedsRebind);
            Err(AccountError::CredentialRollback)
        } else {
            Ok(())
        }
    }

    fn change_credentials(
        &self,
        changes: &[CredentialChange],
    ) -> Result<Vec<CredentialChange>, AccountError> {
        let snapshot = changes
            .iter()
            .map(|(account, _)| Ok((account.clone(), self.secrets.get(account)?)))
            .collect::<Result<Vec<_>, AccountError>>()?;
        for (account, value) in changes {
            let changed = match value {
                Some(value) => self.secrets.set(account, value),
                None => self.secrets.delete(account),
            };
            if let Err(error) = changed {
                self.restore_credentials(&snapshot)?;
                return Err(error.into());
            }
        }
        Ok(snapshot)
    }

    fn clear_user_data(conn: &Connection) -> Result<(), AccountError> {
        // Takeover backups/local client credentials are device state, not another account's data.
        conn.execute_batch(
            "DELETE FROM aicli_cache; DELETE FROM aicli_profile;
            DELETE FROM aicli_scope_binding; DELETE FROM aicli_managed_token;
            DELETE FROM aicli_usage_daily;",
        )
        .map_err(|_| AccountError::Database)
    }

    pub async fn bind(
        &self,
        access: &str,
        selected_line: &str,
    ) -> Result<AccountInfo, AccountError> {
        let _operation = self.operation.lock().await;
        let access = access.trim();
        if access.starts_with("sk-") {
            return Err(AccountError::WrongCredentialType);
        }
        if access.is_empty() {
            return Err(NewApiError::InvalidInput.into());
        }
        if line(selected_line).is_none() {
            return Err(NewApiError::UnknownLine.into());
        }
        // Validation completes before touching storage, even when replacing an existing binding.
        let verified = self.remote.verify(access, selected_line).await?;
        let data = verified.get("data").ok_or(NewApiError::InvalidResponse)?;
        let info = AccountInfo {
            user_id: data
                .get("id")
                .and_then(Value::as_i64)
                .filter(|id| *id > 0)
                .ok_or(NewApiError::InvalidResponse)?,
            username: data
                .get("username")
                .and_then(Value::as_str)
                .ok_or(NewApiError::InvalidResponse)?
                .to_owned(),
            user_group: data
                .get("group")
                .and_then(Value::as_str)
                .ok_or(NewApiError::InvalidResponse)?
                .to_owned(),
            selected_line_id: selected_line.to_owned(),
        };
        {
            let mut conn = self.db.conn.lock().map_err(|_| AccountError::Database)?;
            let previous = conn
                .query_row("SELECT user_id FROM aicli_account WHERE id=1", [], |row| {
                    row.get::<_, i64>(0)
                })
                .optional()
                .map_err(|_| AccountError::Database)?;
            let changed_user = previous.is_some_and(|id| id != info.user_id);
            let mut changes = Vec::new();
            if changed_user {
                changes.extend(
                    Self::managed_tokens(&conn)?
                        .into_iter()
                        .map(|(id, _)| (format!("token:{id}"), None)),
                );
            }
            changes.push(("access-token".to_owned(), Some(access.to_owned())));
            let transaction = conn.transaction().map_err(|_| AccountError::Database)?;
            if changed_user {
                Self::clear_user_data(&transaction)?;
                transaction.execute("UPDATE aicli_account SET active_profile_id=NULL,last_sync_at=NULL WHERE id=1", [])
                    .map_err(|_| AccountError::Database)?;
            }
            transaction.execute("INSERT INTO aicli_account (id,user_id,username,user_group,selected_line_id) VALUES (1,?1,?2,?3,?4)
                ON CONFLICT(id) DO UPDATE SET user_id=excluded.user_id,username=excluded.username,user_group=excluded.user_group,selected_line_id=excluded.selected_line_id",
                rusqlite::params![info.user_id, info.username, info.user_group, info.selected_line_id])
                .map_err(|_| AccountError::Database)?;
            let snapshot = self.change_credentials(&changes)?;
            if transaction.commit().is_err() {
                self.restore_credentials(&snapshot)?;
                return Err(AccountError::Database);
            }
            self.set_state(BindingState::Bound);
        }
        self.lifecycle.refresh();
        Ok(info)
    }

    pub async fn unbind(&self, delete_created_tokens: bool) -> Result<(), AccountError> {
        let _operation = self.operation.lock().await;
        self.lifecycle.release_all().await?;
        let tokens = {
            let conn = self.db.conn.lock().map_err(|_| AccountError::Database)?;
            Self::managed_tokens(&conn)?
        };
        if delete_created_tokens && tokens.iter().any(|(_, created)| *created) {
            let generation = self.generation();
            let info = self.account()?.ok_or(AccountError::NotBound)?;
            let access = self
                .secrets
                .get("access-token")?
                .ok_or(AccountError::NotBound)?;
            for (id, created) in &tokens {
                if *created {
                    let result = self
                        .remote
                        .delete(&access, &info.selected_line_id, *id)
                        .await;
                    if let Err(error) = result {
                        self.observe_api_error(generation, error);
                        return Err(error.into());
                    }
                }
            }
        }
        let mut conn = self.db.conn.lock().map_err(|_| AccountError::Database)?;
        let transaction = conn.transaction().map_err(|_| AccountError::Database)?;
        Self::clear_user_data(&transaction)?;
        transaction
            .execute_batch(
                "DELETE FROM aicli_account; DELETE FROM aicli_local_token;
            DELETE FROM aicli_line_health; DELETE FROM aicli_takeover;",
            )
            .map_err(|_| AccountError::Database)?;
        let mut changes: Vec<_> = tokens
            .into_iter()
            .map(|(id, _)| (format!("token:{id}"), None))
            .collect();
        changes.push(("access-token".to_owned(), None));
        let snapshot = self.change_credentials(&changes)?;
        if transaction.commit().is_err() {
            self.restore_credentials(&snapshot)?;
            return Err(AccountError::Database);
        }
        self.set_state(BindingState::NotBound);
        Ok(())
    }
}

#[cfg(test)]
mod tests;
