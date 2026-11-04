use super::super::secrets::MemoryStore;
use super::*;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Mutex;

const NOW: i64 = 1_800_000_000;

fn order(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

fn candidate(id: i64, name: &str, groups: &[String]) -> Value {
    json!({"id":id,"name":name,"status":1,"expired_time":-1,"unlimited_quota":true,
        "model_limits_enabled":false,"allow_ips":null,"group":if groups.len()==1 {groups[0].as_str()} else {"auto"},
        "auto_groups":groups,"cross_group_retry":true})
}

#[derive(Default)]
struct Remote {
    rows: Mutex<Vec<Value>>,
    creates: AtomicUsize,
    lists: AtomicUsize,
    batches: Mutex<Vec<Vec<i64>>>,
    key_error: Mutex<Option<NewApiError>>,
}

impl TokenRemote for Remote {
    fn list(&self) -> BoxFuture<'_, Result<Vec<Value>, NewApiError>> {
        self.lists.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(self.rows.lock().unwrap().clone()) })
    }
    fn create<'a>(&'a self, specification: &'a Value) -> BoxFuture<'a, Result<(), NewApiError>> {
        Box::pin(async move {
            self.creates.fetch_add(1, Ordering::SeqCst);
            let mut rows = self.rows.lock().unwrap();
            let id = rows
                .iter()
                .filter_map(|v| v["id"].as_i64())
                .max()
                .unwrap_or(0)
                + 1;
            let groups = if specification["group"] == "auto" {
                group_list(&specification["auto_groups"]).unwrap()
            } else {
                vec![specification["group"].as_str().unwrap().into()]
            };
            let mut row = candidate(id, specification["name"].as_str().unwrap(), &groups);
            row.as_object_mut()
                .unwrap()
                .extend(specification.as_object().unwrap().clone());
            rows.push(row);
            Ok(())
        })
    }
    fn keys<'a>(
        &'a self,
        ids: &'a [i64],
    ) -> BoxFuture<'a, Result<BTreeMap<i64, String>, NewApiError>> {
        Box::pin(async move {
            self.batches.lock().unwrap().push(ids.to_vec());
            if let Some(error) = self.key_error.lock().unwrap().take() {
                return Err(error);
            }
            Ok(ids
                .iter()
                .map(|id| (*id, format!("fixture-value-{id}")))
                .collect())
        })
    }
}

fn service(remote: Arc<Remote>) -> (TokenService, Arc<MemoryStore>) {
    let secrets = Arc::new(MemoryStore::default());
    let mut service = TokenService::new(
        Arc::new(Database::memory().unwrap()),
        secrets.clone(),
        remote,
    );
    service.clock = Arc::new(|| NOW);
    (service, secrets)
}

#[test]
fn group_hash_is_exact_ordered_sha256_and_invalid_inputs_are_rejected() {
    let groups = order(&["first", "second"]);
    assert_eq!(
        groups_hash(&groups),
        format!("{:x}", Sha256::digest(b"first\nsecond"))
    );
    assert_ne!(
        groups_hash(&groups),
        groups_hash(&order(&["second", "first"]))
    );
    for groups in [
        vec![],
        order(&[""]),
        order(&["a", "a"]),
        order(&["a\nb"]),
        order(&["a\rb"]),
    ] {
        assert!(!valid_groups(&groups));
    }
}

#[test]
fn reuse_checks_every_restriction_and_requires_cross_group_retry() {
    let groups = order(&["cheap", "mid"]);
    let original = candidate(1, "user", &groups);
    assert!(reusable(&original, &groups, NOW));
    for (field, value) in [
        ("status", json!(2)),
        ("expired_time", json!(NOW + REUSE_LIFETIME)),
        ("unlimited_quota", json!(false)),
        ("model_limits_enabled", json!(true)),
        ("allow_ips", json!("127.0.0.1")),
        ("cross_group_retry", json!(false)),
        ("group", json!("cheap")),
        ("auto_groups", json!(["mid", "cheap"])),
    ] {
        let mut changed = original.clone();
        changed[field] = value;
        assert!(!reusable(&changed, &groups, NOW), "restriction {field}");
    }
    let mut expires = original.clone();
    expires["expired_time"] = json!(NOW + REUSE_LIFETIME + 1);
    assert!(reusable(&expires, &groups, NOW));
    expires["auto_groups"] = json!("[\"cheap\",\"mid\"]");
    assert!(reusable(&expires, &groups, NOW));
    let single = order(&["cheap"]);
    let mut one = candidate(2, "user", &single);
    one["cross_group_retry"] = json!(false);
    assert!(reusable(&one, &single, NOW));
    assert!(!reusable(&one, &order(&["mid"]), NOW));
}

#[tokio::test]
async fn stable_reuse_prefers_aicli_prefix_then_smallest_id_without_claiming_ownership() {
    let remote = Arc::new(Remote::default());
    let groups = order(&["cheap", "mid"]);
    *remote.rows.lock().unwrap() = vec![
        candidate(1, "user", &groups),
        candidate(7, "aicli:second", &groups),
        candidate(5, "aicli:first", &groups),
    ];
    let (service, secrets) = service(remote.clone());
    assert_eq!(service.ensure_token(&groups, true, &[]).await.unwrap(), 5);
    assert_eq!(remote.creates.load(Ordering::SeqCst), 0);
    let records = service.records().unwrap();
    assert_eq!(records.len(), 1);
    assert!(!records[0].created);
    assert!(secrets.get("token:5").unwrap().unwrap().starts_with("sk-"));
    assert_eq!(records[0].masked, "sk-…ue-5");
    assert!(!format!("{service:?}").contains("fixture-value"));
}

#[tokio::test]
async fn user_token_reuse_uses_lowest_id_and_never_creates() {
    let remote = Arc::new(Remote::default());
    let groups = order(&["only"]);
    *remote.rows.lock().unwrap() = vec![candidate(9, "z", &groups), candidate(3, "a", &groups)];
    let (service, _) = service(remote.clone());
    assert_eq!(service.ensure_token(&groups, false, &[]).await.unwrap(), 3);
    assert_eq!(remote.creates.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn incompatible_existing_token_is_not_mutated_and_name_collision_gets_a_suffix() {
    let remote = Arc::new(Remote::default());
    let groups = order(&["cheap", "mid"]);
    let mut existing = candidate(1, &format!("aicli:{}", &groups_hash(&groups)[..8]), &groups);
    existing["cross_group_retry"] = json!(false);
    remote.rows.lock().unwrap().push(existing.clone());
    let (service, _) = service(remote.clone());
    let id = service.ensure_token(&groups, true, &[]).await.unwrap();
    assert_eq!(id, 2);
    assert!(service.records().unwrap()[0].created);
    let rows = remote.rows.lock().unwrap();
    assert_eq!(rows[0], existing);
    assert_eq!(
        rows[1]["name"],
        format!("aicli:{}-2", &groups_hash(&groups)[..8])
    );
    assert_eq!(rows[1]["auto_groups"], json!(groups));
    assert_eq!(rows[1]["cross_group_retry"], true);
}

#[tokio::test]
async fn sixty_nine_models_batch_missing_keys_once_and_repeat_or_reopen_is_local() {
    let remote = Arc::new(Remote::default());
    let (service, secrets) = service(remote.clone());
    let clusters = [
        order(&["gpt", "gpt-max", "gpt-pro"]),
        order(&["claude-aws", "claude-code", "claude-kiro"]),
        order(&["gemini-cli"]),
        order(&["xAI"]),
    ];
    let requests: Vec<_> = (0..69).map(|i| clusters[i % 4].clone()).collect();
    let first = service
        .ensure_compilation(&requests, true, &requests)
        .await
        .unwrap();
    assert_eq!(first.len(), 4);
    assert_eq!(remote.creates.load(Ordering::SeqCst), 4);
    assert_eq!(remote.batches.lock().unwrap().len(), 1);
    assert_eq!(remote.batches.lock().unwrap()[0].len(), 4);
    let lists = remote.lists.load(Ordering::SeqCst);
    assert_eq!(
        service
            .ensure_compilation(&requests, true, &requests)
            .await
            .unwrap(),
        first
    );
    let reopened = TokenService::new(service.db.clone(), secrets.clone(), remote.clone());
    assert_eq!(
        reopened
            .ensure_compilation(&requests, true, &requests)
            .await
            .unwrap(),
        first
    );
    assert_eq!(remote.lists.load(Ordering::SeqCst), lists);
    assert_eq!(remote.batches.lock().unwrap().len(), 1);
    let storage: String = service
        .db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT group_concat(groups_json||name||masked,'|') FROM aicli_managed_token",
            [],
            |row| row.get(0),
        )
        .unwrap();
    for id in first.values() {
        assert!(!storage.contains(&secrets.get(&format!("token:{id}")).unwrap().unwrap()));
    }
}

#[tokio::test]
async fn concurrent_compilations_create_and_fetch_only_once() {
    let remote = Arc::new(Remote::default());
    let (service, _) = service(remote.clone());
    let groups = order(&["one"]);
    let (a, b) = tokio::join!(
        service.ensure_token(&groups, false, &[]),
        service.ensure_token(&groups, false, &[])
    );
    assert_eq!(a.unwrap(), b.unwrap());
    assert_eq!(remote.creates.load(Ordering::SeqCst), 1);
    assert_eq!(remote.batches.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn multi_group_unavailable_and_invalid_orders_do_no_http_even_with_cached_key() {
    let remote = Arc::new(Remote::default());
    let (service, _) = service(remote.clone());
    let groups = order(&["a", "b"]);
    service.ensure_token(&groups, true, &[]).await.unwrap();
    let before = remote.lists.load(Ordering::SeqCst);
    assert_eq!(
        service.ensure_token(&groups, false, &[]).await,
        Err(TokenError::AutoGroupUnavailable)
    );
    assert_eq!(
        service.ensure_token(&[], true, &[]).await,
        Err(TokenError::InvalidGroups)
    );
    assert_eq!(
        service.ensure_token(&order(&["a", "a"]), true, &[]).await,
        Err(TokenError::InvalidGroups)
    );
    assert_eq!(remote.lists.load(Ordering::SeqCst), before);
}

#[tokio::test]
async fn missing_secret_is_batched_again_and_keeps_created_ownership() {
    let remote = Arc::new(Remote::default());
    let (service, secrets) = service(remote.clone());
    let groups = order(&["a"]);
    let id = service.ensure_token(&groups, true, &[]).await.unwrap();
    secrets.delete(&format!("token:{id}")).unwrap();
    assert_eq!(service.ensure_token(&groups, true, &[]).await.unwrap(), id);
    assert!(service.records().unwrap()[0].created);
    assert_eq!(remote.creates.load(Ordering::SeqCst), 1);
    assert_eq!(remote.batches.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn rate_limit_defers_without_looping_and_resumes_with_original_ownership() {
    let remote = Arc::new(Remote::default());
    let (mut service, _) = service(remote.clone());
    let groups = order(&["a"]);
    let time = Arc::new(AtomicI64::new(NOW));
    let clock = time.clone();
    service.clock = Arc::new(move || clock.load(Ordering::SeqCst));
    *remote.key_error.lock().unwrap() = Some(NewApiError::RateLimited(91));
    assert_eq!(
        service.ensure_token(&groups, true, &[]).await,
        Err(TokenError::RateLimited { retry_at: NOW + 91 })
    );
    assert!(service.records().unwrap()[0].created);
    let lists = remote.lists.load(Ordering::SeqCst);
    assert_eq!(
        service.ensure_token(&groups, true, &[]).await,
        Err(TokenError::RateLimited { retry_at: NOW + 91 })
    );
    assert_eq!(remote.lists.load(Ordering::SeqCst), lists);
    assert_eq!(remote.batches.lock().unwrap().len(), 1);
    time.store(NOW + 91, Ordering::SeqCst);
    service.ensure_token(&groups, true, &[]).await.unwrap();
    assert_eq!(remote.creates.load(Ordering::SeqCst), 1);
    assert!(service.records().unwrap()[0].created);
}

#[tokio::test]
async fn rate_limit_without_header_uses_a_bounded_default_deferral() {
    let remote = Arc::new(Remote::default());
    let (service, _) = service(remote.clone());
    *remote.key_error.lock().unwrap() = Some(NewApiError::Http(429));
    assert_eq!(
        service.ensure_token(&order(&["a"]), true, &[]).await,
        Err(TokenError::RateLimited { retry_at: NOW + 60 })
    );
    assert_eq!(remote.batches.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn limit_never_cleans_remote_tokens_and_excludes_all_profile_references() {
    let remote = Arc::new(Remote::default());
    let (service, _) = service(remote.clone());
    for id in 1..=32 {
        remote.rows.lock().unwrap().push(candidate(
            id,
            &format!("aicli:fixture-{id}"),
            &order(&[&format!("g{id}")]),
        ));
        service
            .save(&Record {
                id,
                hash: groups_hash(&order(&[&format!("g{id}")])),
                groups: order(&[&format!("g{id}")]),
                name: format!("aicli:fixture-{id}"),
                masked: format!("sk-…{id:04}"),
                created: true,
            })
            .unwrap();
    }
    let result = service
        .ensure_token(&order(&["new"]), true, &[order(&["g1"]), order(&["g2"])])
        .await;
    let Err(TokenError::TokenLimit { unused }) = result else {
        panic!("expected TokenLimit")
    };
    assert_eq!(unused.len(), 30);
    assert!(!unused
        .iter()
        .any(|r| r.name == "aicli:fixture-1" || r.name == "aicli:fixture-2"));
    assert_eq!(remote.creates.load(Ordering::SeqCst), 0);
    assert_eq!(service.records().unwrap().len(), 32);
    let serialized = serde_json::to_string(&unused).unwrap();
    assert!(!serialized.contains("token_id"));
    remote
        .rows
        .lock()
        .unwrap()
        .push(candidate(40, "user-existing", &order(&["new"])));
    assert_eq!(
        service
            .ensure_token(&order(&["new"]), true, &[])
            .await
            .unwrap(),
        40
    );
    assert_eq!(remote.creates.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn deleted_token_recovers_once_and_clears_old_local_credential() {
    let remote = Arc::new(Remote::default());
    let (service, secrets) = service(remote.clone());
    let groups = order(&["a"]);
    let first = service.ensure_token(&groups, true, &[]).await.unwrap();
    remote.rows.lock().unwrap().clear();
    // Keep a higher irrelevant ID, as the real server never reuses a deleted ID.
    remote
        .rows
        .lock()
        .unwrap()
        .push(candidate(5, "other", &order(&["b"])));
    let second = service
        .recover_unauthorized(first, &groups, true, &[])
        .await
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(secrets.get(&format!("token:{first}")).unwrap(), None);
    assert_eq!(service.records().unwrap().len(), 1);
    assert_eq!(
        service
            .recover_unauthorized(first, &groups, true, &[])
            .await
            .unwrap(),
        second
    );
    assert_eq!(remote.creates.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn failed_recovery_is_not_repeated_in_a_loop() {
    let remote = Arc::new(Remote::default());
    let (service, _) = service(remote.clone());
    let groups = order(&["a"]);
    let id = service.ensure_token(&groups, true, &[]).await.unwrap();
    *remote.key_error.lock().unwrap() = Some(NewApiError::Rejected);
    assert_eq!(
        service.recover_unauthorized(id, &groups, true, &[]).await,
        Err(TokenError::Api(NewApiError::Rejected))
    );
    let batches = remote.batches.lock().unwrap().len();
    assert_eq!(
        service.recover_unauthorized(id, &groups, true, &[]).await,
        Err(TokenError::RecoveryExhausted)
    );
    assert_eq!(remote.batches.lock().unwrap().len(), batches);
}
