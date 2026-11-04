//! Bound, offline fixtures for the existing forwarding regressions.
use super::{
    newapi::{NewApiClient, NewApiError},
    runtime::{AicliRuntime, RuntimeRemote, RuntimeValues},
    secrets::{MemoryStore, SecretStore},
    tokens::{groups_hash, TokenRemote},
};
use futures::future::BoxFuture;
use serde_json::{json, Value};
use std::{collections::BTreeMap, sync::Arc};

struct Offline;
impl RuntimeRemote for Offline {
    fn fetch<'a>(
        &'a self,
        _: &'a NewApiClient,
    ) -> BoxFuture<'a, Result<RuntimeValues, NewApiError>> {
        Box::pin(async { Err(NewApiError::Network) })
    }
    fn tokens(&self, _: NewApiClient) -> Arc<dyn TokenRemote> {
        Arc::new(Self)
    }
}
impl TokenRemote for Offline {
    fn list(&self) -> BoxFuture<'_, Result<Vec<Value>, NewApiError>> {
        Box::pin(async { Err(NewApiError::Network) })
    }
    fn create<'a>(&'a self, _: &'a Value) -> BoxFuture<'a, Result<(), NewApiError>> {
        Box::pin(async { Err(NewApiError::Network) })
    }
    fn keys<'a>(
        &'a self,
        _: &'a [i64],
    ) -> BoxFuture<'a, Result<BTreeMap<i64, String>, NewApiError>> {
        Box::pin(async { Err(NewApiError::Network) })
    }
}

pub(crate) fn bound_provider_runtime(db: Arc<crate::Database>) -> Arc<AicliRuntime> {
    let store = Arc::new(MemoryStore::default());
    store
        .set("access-token", "synthetic-forwarding-access")
        .unwrap();
    store
        .set("token:900", "synthetic-forwarding-model-key")
        .unwrap();
    let models = [
        "gpt-5",
        "gpt-image-1",
        "gpt-image-2",
        "gpt-5.6-sol",
        "seed-model",
        "claude-sonnet-4-6",
    ];
    let pricing = json!({"data":models.into_iter().map(|model| json!({"model_name":model,"enable_groups":["fixture"],"supported_endpoint_types":["anthropic","openai","openai-response","gemini"]})).collect::<Vec<_>>()});
    {
        let conn = db.conn.lock().unwrap();
        conn.execute("INSERT INTO aicli_account(id,user_id,username,user_group) VALUES(1,900,'fixture','fixture')", []).unwrap();
        for (kind, value) in [
            ("groups", json!({"data":{"fixture":{"ratio":1}}})),
            ("pricing", pricing),
        ] {
            conn.execute(
                "INSERT INTO aicli_cache(kind,fetched_at,body_json) VALUES(?1,1,?2)",
                rusqlite::params![kind, value.to_string()],
            )
            .unwrap();
        }
        conn.execute(
            r#"INSERT INTO aicli_managed_token VALUES(900,?1,'["fixture"]','fixture','masked',0)"#,
            [groups_hash(&["fixture".into()])],
        )
        .unwrap();
    }
    let runtime = AicliRuntime::with_remote(db.clone(), store, Arc::new(Offline));
    runtime.set_local_test_provider_source(move |app, _, _| {
        let id = db.get_current_provider(app.as_str()).unwrap()?;
        db.get_provider_by_id(&id, app.as_str()).unwrap()
    });
    runtime
}
