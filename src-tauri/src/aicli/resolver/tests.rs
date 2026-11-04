use super::*;
use crate::aicli::{
    newapi::{NewApiClient, NewApiError},
    profiles::{ModelBinding, ProfileStore},
    runtime::{RuntimeRemote, RuntimeValues},
    secrets::{MemoryStore, SecretStore},
    tokens::{groups_hash, TokenRemote},
};
use futures::future::BoxFuture;
use serde_json::json;
use std::{collections::BTreeMap, sync::Mutex, time::Duration};

const KEY: &str = "synthetic-resolver-credential";
const SESSION: &str = "resolver-session-000000000001";

fn values(model: &str) -> RuntimeValues {
    (
        json!({"data":{"id":7}}),
        json!({"data":{"auto":{},"group-a":{"ratio":1}}}),
        json!({"data":[{"model_name":model,"enable_groups":["group-a"],"supported_endpoint_types":["anthropic","openai","openai-response","gemini"]}]}),
        json!({"data":{"groups":["group-a"],"max_count":10}}),
    )
}

struct Remote(Mutex<RuntimeValues>);
struct Tokens;
impl TokenRemote for Tokens {
    fn list(&self) -> BoxFuture<'_, Result<Vec<Value>, NewApiError>> {
        Box::pin(async {
            Ok(vec![
                json!({"id":17,"name":"fixture","group":"group-a","status":1,"expired_time":-1,"unlimited_quota":true,"model_limits_enabled":false,"allow_ips":null}),
            ])
        })
    }
    fn create<'a>(&'a self, _: &'a Value) -> BoxFuture<'a, Result<(), NewApiError>> {
        Box::pin(async { panic!("fixture token must be reused") })
    }
    fn keys<'a>(
        &'a self,
        _: &'a [i64],
    ) -> BoxFuture<'a, Result<BTreeMap<i64, String>, NewApiError>> {
        Box::pin(async { Ok(BTreeMap::from([(17, KEY.into())])) })
    }
}
impl RuntimeRemote for Remote {
    fn fetch<'a>(
        &'a self,
        _: &'a NewApiClient,
    ) -> BoxFuture<'a, Result<RuntimeValues, NewApiError>> {
        Box::pin(async { Ok(self.0.lock().unwrap().clone()) })
    }
    fn tokens(&self, _: NewApiClient) -> Arc<dyn TokenRemote> {
        Arc::new(Tokens)
    }
}

struct Fixture {
    db: Arc<crate::Database>,
    store: Arc<MemoryStore>,
    runtime: Arc<AicliRuntime>,
    remote: Arc<Remote>,
}

impl Fixture {
    fn new() -> Self {
        let db = Arc::new(crate::Database::memory().unwrap());
        let store = Arc::new(MemoryStore::default());
        store
            .set("access-token", "synthetic-resolver-access")
            .unwrap();
        store.set("token:17", KEY).unwrap();
        db.conn.lock().unwrap().execute("INSERT INTO aicli_account(id,user_id,username,user_group) VALUES(1,7,'fixture','group-a')", []).unwrap();
        let snapshots = values("model-a");
        for (kind, body) in [
            ("groups", &snapshots.1),
            ("pricing", &snapshots.2),
            ("auto_groups", &snapshots.3),
        ] {
            db.conn
                .lock()
                .unwrap()
                .execute(
                    "INSERT INTO aicli_cache(kind,fetched_at,body_json) VALUES(?1,1,?2)",
                    rusqlite::params![kind, body.to_string()],
                )
                .unwrap();
        }
        db.conn.lock().unwrap().execute("INSERT INTO aicli_managed_token VALUES(17,?1,'[\"group-a\"]','fixture','masked',0)",[groups_hash(&["group-a".into()])]).unwrap();
        let remote = Arc::new(Remote(Mutex::new(snapshots)));
        let runtime = AicliRuntime::with_remote(db.clone(), store.clone(), remote.clone());
        Self {
            db,
            store,
            runtime,
            remote,
        }
    }

    fn reload(&mut self) {
        self.runtime =
            AicliRuntime::with_remote(self.db.clone(), self.store.clone(), self.remote.clone());
    }

    fn profile(&self, id: &str, policy: &str, alias: Option<&str>) {
        let profiles = ProfileStore::new(self.db.clone());
        let snapshot = self.runtime.snapshot();
        let catalog = snapshot.catalog.as_ref().unwrap();
        let mut profile = profiles
            .copy_preset("recommended", id, id, catalog)
            .unwrap();
        profile.body.line_policy = policy.into();
        if let Some(alias) = alias {
            profile.body.model_bindings.push(ModelBinding {
                model: "model-a".into(),
                source_group: Some("group-a".into()),
                alias: Some(alias.into()),
            });
        }
        profiles.save(&profile, catalog).unwrap();
    }

    fn scope(&self, scope: &str, key: &str, profile: &str) {
        self.db
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT OR REPLACE INTO aicli_scope_binding VALUES(?1,?2,?3)",
                [scope, key, profile],
            )
            .unwrap();
    }

    fn active(&self, profile: Option<&str>) {
        self.db
            .conn
            .lock()
            .unwrap()
            .execute("UPDATE aicli_account SET active_profile_id=?1", [profile])
            .unwrap();
    }

    fn pinned(&self, change: impl FnOnce(&mut RoutesSnapshot)) {
        let mut snapshot = (*self.runtime.snapshot()).clone();
        change(&mut snapshot);
        self.runtime.pin(
            serde_json::to_string(&("claude", SESSION)).unwrap(),
            "recommended".into(),
            Arc::new(snapshot),
        );
    }

    fn resolve(&self, model: &str, session: bool) -> (Resolved, Value) {
        let mut body = json!({"model": model});
        let mut headers = HeaderMap::new();
        if session {
            headers.insert("x-claude-code-session-id", SESSION.parse().unwrap());
        }
        let resolved = resolve(
            &self.runtime,
            &AppType::Claude,
            "anthropic",
            &headers,
            "/v1/messages",
            &mut body,
        )
        .unwrap();
        (resolved, body)
    }
}

fn hit(resolved: Resolved) -> Box<RouteDecision> {
    match resolved {
        Resolved::Hit(decision) => decision,
        other => panic!("expected hit, got {other:?}"),
    }
}

#[test]
fn unbound_missing_secret_and_rebound_accounts_cannot_use_stale_routes() {
    for case in ["deleted", "secret", "rotated", "rebound"] {
        let f = Fixture::new();
        assert!(matches!(f.resolve("model-a", false).0, Resolved::Hit(_)));
        match case {
            "deleted" => {
                f.db.conn
                    .lock()
                    .unwrap()
                    .execute("DELETE FROM aicli_account", [])
                    .unwrap();
            }
            "secret" => f.store.delete("access-token").unwrap(),
            "rotated" => f
                .store
                .set("access-token", "synthetic-rotated-access")
                .unwrap(),
            _ => {
                f.db.conn
                    .lock()
                    .unwrap()
                    .execute("UPDATE aicli_account SET user_id=8", [])
                    .unwrap();
            }
        }
        assert!(
            matches!(f.resolve("model-a", false).0, Resolved::NotBound),
            "{case}"
        );
    }
}

#[test]
fn plain_requests_and_absent_models_do_not_mutate_input() {
    let f = Fixture::new();
    for mut body in [json!({}), json!({"model":""})] {
        let original = body.clone();
        assert!(matches!(
            resolve(
                &f.runtime,
                &AppType::Claude,
                "anthropic",
                &HeaderMap::new(),
                "/v1/messages",
                &mut body
            )
            .unwrap(),
            Resolved::Plain
        ));
        assert_eq!(body, original);
    }
    let (resolved, body) = f.resolve("missing", true);
    assert!(matches!(
        resolved,
        Resolved::Miss(RouteMiss::ModelNotInProfile)
    ));
    assert_eq!(body["model"], "missing");
    assert!(f
        .runtime
        .session_pin(&serde_json::to_string(&("claude", SESSION)).unwrap())
        .is_none());
}

#[test]
fn aliases_are_decoded_and_upstream_model_is_rewritten_without_keys_in_providers() {
    let mut f = Fixture::new();
    f.profile("custom", "inherit", Some("friendly"));
    f.active(Some("custom"));
    f.reload();
    let encoded = alias::encode(&AppType::Claude, "friendly", Some(&f.db)).unwrap();
    let (resolved, body) = f.resolve(&encoded, true);
    let decision = hit(resolved);
    assert_eq!(decision.public_model, "friendly");
    assert_eq!(decision.upstream_model, "model-a");
    assert_eq!(body["model"], "model-a");
    assert_eq!(decision.credential, KEY);
    assert_eq!(decision.token_id, 17);
    assert_eq!(decision.session_key, format!("custom:{SESSION}"));
    assert_eq!(decision.wire_protocol, "anthropic");
    assert!(!format!("{decision:?}").contains(KEY));
    assert!(decision
        .providers
        .iter()
        .all(|p| !serde_json::to_string(p).unwrap().contains(KEY)));
}

#[test]
fn desktop_alias_uses_the_persisted_alias_map() {
    let f = Fixture::new();
    let model = alias::encode(&AppType::ClaudeDesktop, "model-a", Some(&f.db)).unwrap();
    let mut body = json!({"model": model});
    let decision = hit(resolve(
        &f.runtime,
        &AppType::ClaudeDesktop,
        "anthropic",
        &HeaderMap::new(),
        "/v1/messages",
        &mut body,
    )
    .unwrap());
    assert_eq!(decision.public_model, "model-a");
    assert_eq!(body["model"], "model-a");
}

#[test]
fn scope_order_is_session_then_app_then_global_then_active_then_recommended() {
    for expected in ["session", "app", "global", "active", "recommended"] {
        let mut f = Fixture::new();
        for name in ["session", "app", "global", "active", "project"] {
            f.profile(name, "inherit", None);
        }
        f.scope("project", "/some/project", "project");
        if expected != "recommended" {
            f.active(Some("active"));
        }
        if ["session", "app", "global"].contains(&expected) {
            f.scope("global", "", "global");
        }
        if ["session", "app"].contains(&expected) {
            f.scope("app", "claude", "app");
        }
        if expected == "session" {
            f.scope("session", SESSION, "session");
        }
        f.reload();
        assert_eq!(hit(f.resolve("model-a", true).0).profile_id, expected);
    }
}

#[tokio::test]
async fn pin_wins_over_later_scope_active_and_catalog_changes() {
    let mut f = Fixture::new();
    f.profile("new", "apac", None);
    f.reload();
    let first = hit(f.resolve("model-a", true).0);
    f.scope("session", SESSION, "new");
    f.active(Some("new"));
    *f.remote.0.lock().unwrap() = values("model-b");
    f.runtime.refresh().await.unwrap();
    assert!(f
        .runtime
        .snapshot()
        .catalog
        .as_ref()
        .unwrap()
        .models
        .contains_key("model-b"));
    let pinned = hit(f.resolve("model-a", true).0);
    assert_eq!(pinned.profile_id, first.profile_id);
    assert_eq!(pinned.upstream_model, first.upstream_model);
    assert_eq!(pinned.credential, first.credential);
    assert_eq!(pinned.providers[0].id, first.providers[0].id);
    assert!(matches!(
        f.resolve("model-a", false).0,
        Resolved::Miss(RouteMiss::ModelNotInProfile)
    ));
}

#[test]
fn generated_sessions_never_pin_or_emit_affinity_and_reselect_each_request() {
    let mut f = Fixture::new();
    f.profile("new", "apac", None);
    f.reload();
    assert!(hit(f.resolve("model-a", false).0).session_key.is_empty());
    f.scope("app", "claude", "new");
    let next = hit(f.resolve("model-a", false).0);
    assert_eq!(next.profile_id, "new");
    assert!(next.session_key.is_empty());
    let mut body = json!({"contents":[]});
    let gemini = hit(resolve(
        &f.runtime,
        &AppType::Gemini,
        "gemini",
        &HeaderMap::new(),
        "/v1beta/models/model-a:generateContent?key=synthetic-query",
        &mut body,
    )
    .unwrap());
    assert_eq!(gemini.public_model, "model-a");
    assert_eq!(body["model"], "model-a");
    assert!(gemini.session_key.is_empty());
}

#[test]
fn codex_uses_the_existing_session_extractor_and_protocol_specific_wire() {
    let f = Fixture::new();
    let mut headers = HeaderMap::new();
    headers.insert("session-id", SESSION.parse().unwrap());
    for protocol in ["openai", "openai-response"] {
        let mut body = json!({"model":"model-a"});
        let decision = hit(resolve(
            &f.runtime,
            &AppType::Codex,
            protocol,
            &headers,
            "/v1/responses",
            &mut body,
        )
        .unwrap());
        assert_eq!(decision.session_key, format!("recommended:codex_{SESSION}"));
        assert_eq!(decision.wire_protocol, protocol);
    }
}

#[test]
fn repair_precedes_model_lookup_and_only_blocks_affected_models() {
    let f = Fixture::new();
    f.pinned(|snapshot| {
        let repair = &mut snapshot
            .profiles
            .get_mut("recommended")
            .unwrap()
            .compiled
            .repair;
        repair.needs_repair = true;
        repair.affected_models = vec!["revoked-exclusive-model".into()];
    });
    assert!(matches!(
        f.resolve("revoked-exclusive-model", true).0,
        Resolved::Miss(RouteMiss::NeedsRepair)
    ));
    assert!(matches!(f.resolve("model-a", true).0, Resolved::Hit(_)));
    f.pinned(|snapshot| {
        let repair = &mut snapshot
            .profiles
            .get_mut("recommended")
            .unwrap()
            .compiled
            .repair;
        repair.needs_repair = true;
        repair.conservative = true;
        repair.affected_models = vec!["model-a".into()];
    });
    assert!(matches!(
        f.resolve("model-a", true).0,
        Resolved::Miss(RouteMiss::NeedsRepair)
    ));
}

#[test]
fn table_of_missing_credentials_wire_catalog_and_unavailable_capability() {
    for (case, expected) in [
        ("token", RouteMiss::NotBound),
        ("key", RouteMiss::NotBound),
        ("wire", RouteMiss::Unsupported),
        ("catalog", RouteMiss::Unsupported),
        ("unavailable", RouteMiss::Unsupported),
        ("profile", RouteMiss::ModelNotInProfile),
    ] {
        let f = Fixture::new();
        f.pinned(|snapshot| match case {
            "token" => {
                snapshot
                    .profiles
                    .get_mut("recommended")
                    .unwrap()
                    .token_ids
                    .clear();
            }
            "key" => {
                snapshot
                    .profiles
                    .get_mut("recommended")
                    .unwrap()
                    .token_ids
                    .insert("model-a".into(), 999);
            }
            "wire" => {
                snapshot
                    .catalog
                    .as_mut()
                    .unwrap()
                    .models
                    .get_mut("model-a")
                    .unwrap()
                    .wire
                    .clear();
            }
            "catalog" => {
                snapshot.catalog = None;
            }
            "unavailable" => {
                snapshot
                    .catalog
                    .as_mut()
                    .unwrap()
                    .models
                    .get_mut("model-a")
                    .unwrap()
                    .compat
                    .insert("anthropic".into(), Compatibility::Unavailable);
            }
            _ => {
                snapshot.profiles.clear();
            }
        });
        assert!(
            matches!(f.resolve("model-a", true).0, Resolved::Miss(miss) if miss == expected),
            "{case}"
        );
    }
}

#[test]
fn fixed_lines_preference_priority_timeouts_and_two_level_cooldowns() {
    let mut f = Fixture::new();
    f.profile("custom", "global", None);
    f.active(Some("custom"));
    f.db.conn
        .lock()
        .unwrap()
        .execute("UPDATE aicli_account SET selected_line_id='apac'", [])
        .unwrap();
    f.reload();
    let decision = hit(f.resolve("model-a", false).0);
    assert_eq!(
        decision
            .providers
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["aicli-global", "aicli-main", "aicli-apac"]
    );
    assert_eq!(decision.timeouts.len(), decision.providers.len());
    for line in &LINES {
        let timeout = decision.timeouts[&format!("aicli-{}", line.id)];
        assert_eq!(timeout.first_byte_s, line.first_byte_s);
        assert_eq!(timeout.idle_s, line.idle_s);
        assert_eq!(timeout.stream_total_s, line.stream_total_s);
        assert_eq!(timeout.non_stream_s, line.non_stream_s);
    }
    let now = Instant::now();
    f.runtime
        .cooldown
        .lines
        .lock()
        .unwrap()
        .insert("global".into(), now + Duration::from_secs(90));
    f.runtime.cooldown.models.lock().unwrap().insert(
        ("main".into(), "model-a".into()),
        now + Duration::from_secs(60),
    );
    assert_eq!(
        hit(f.resolve("model-a", false).0).providers[0].id,
        "aicli-apac"
    );
    f.runtime
        .cooldown
        .lines
        .lock()
        .unwrap()
        .insert("apac".into(), now - Duration::from_secs(1));
    f.runtime.cooldown.models.lock().unwrap().insert(
        ("apac".into(), "other-model".into()),
        now + Duration::from_secs(60),
    );
    assert_eq!(hit(f.resolve("model-a", false).0).providers.len(), 1);
    f.active(None);
    f.reload();
    assert_eq!(
        hit(f.resolve("model-a", false).0).providers[0].id,
        "aicli-apac"
    );
}

#[test]
fn all_cooling_probes_only_earliest_effective_recovery() {
    let f = Fixture::new();
    let now = Instant::now();
    for (line, seconds) in [("main", 10), ("global", 20), ("apac", 30)] {
        f.runtime
            .cooldown
            .lines
            .lock()
            .unwrap()
            .insert(line.into(), now + Duration::from_secs(seconds));
    }
    // main's line timeout expires first, but its model timeout expires last.
    f.runtime.cooldown.models.lock().unwrap().insert(
        ("main".into(), "model-a".into()),
        now + Duration::from_secs(90),
    );
    let decision = hit(f.resolve("model-a", false).0);
    assert_eq!(decision.providers.len(), 1);
    assert_eq!(decision.providers[0].id, "aicli-global");
    assert_eq!(decision.timeouts.len(), 1);
}

#[test]
fn protocol_error_envelopes_have_distinct_codes_and_bind_is_unauthorized() {
    for app in [
        AppType::Claude,
        AppType::ClaudeDesktop,
        AppType::Codex,
        AppType::Gemini,
    ] {
        for miss in [
            RouteMiss::NotBound,
            RouteMiss::ModelNotInProfile,
            RouteMiss::NeedsRepair,
            RouteMiss::Unsupported,
        ] {
            let (status, body) = miss.response(&app);
            assert_eq!(body["error"]["code"], miss.code());
            assert_eq!(
                status,
                if miss == RouteMiss::NotBound {
                    StatusCode::UNAUTHORIZED
                } else {
                    StatusCode::BAD_REQUEST
                }
            );
            if matches!(app, AppType::Claude | AppType::ClaudeDesktop) {
                assert_eq!(body["type"], "error");
            } else {
                assert_eq!(body["error"]["param"], "model");
            }
            assert!(!body.to_string().contains(KEY));
        }
    }
}
