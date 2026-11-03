use super::*;
use serde_json::json;

fn groups() -> Value {
    json!({"data":{"g-cheap":{"ratio":0.5,"desc":"cheap"},"g-mid":{"ratio":2},"unknown":{},"auto":{"ratio":"自动"}}})
}

fn pricing(endpoints: Value) -> Value {
    json!({"data":[{"model_name":"model-a","enable_groups":["svip","auto","g-mid","g-cheap","g-mid"],
        "supported_endpoint_types":endpoints,"billing_mode":"tiered_expr","billing_expr":"server-expression",
        "quota_type":1,"model_price":0.25,"future_field":{"keep":true}}],
        "auto_groups":["svip","g-mid","g-cheap","g-mid","auto"]})
}

#[test]
fn permissions_intersect_and_exclude_auto_and_unavailable_groups() {
    let catalog = Catalog::from_snapshots(&groups(), &pricing(json!(["openai"]))).unwrap();
    assert!(catalog.auto_available);
    assert!(!catalog.groups.contains_key("auto"));
    assert!(!catalog.groups.contains_key("svip"));
    assert_eq!(catalog.models["model-a"].groups, ["g-cheap", "g-mid"]);
    assert_eq!(catalog.auto_groups, ["g-mid", "g-cheap"]);
    assert_eq!(catalog.groups["g-cheap"].ratio, Some(0.5));
    assert!(catalog.groups["g-cheap"].ratio_known);
    assert_eq!(catalog.groups["unknown"].ratio, None);
    assert!(!catalog.groups["unknown"].ratio_known);
}

#[test]
fn missing_ratio_never_uses_pricing_ratio_or_defaults_to_one() {
    let mut groups = groups();
    groups["data"]["g-mid"]["ratio"] = json!("unknown");
    groups["data"].as_object_mut().unwrap().remove("auto");
    let mut pricing = pricing(json!(["openai"]));
    pricing["group_ratio"] = json!({"g-mid":1});
    let catalog = Catalog::from_snapshots(&groups, &pricing).unwrap();
    assert!(!catalog.auto_available);
    assert_eq!(catalog.groups["g-mid"].ratio, None);
    assert!(!catalog.groups["g-mid"].ratio_known);
}

#[test]
fn preserves_all_server_billing_metadata_without_evaluating_expressions() {
    let pricing = pricing(json!(["openai", "anthropic"]));
    let catalog = Catalog::from_snapshots(&groups(), &pricing).unwrap();
    assert_eq!(catalog.models["model-a"].pricing, pricing["data"][0]);
}

#[test]
fn excludes_models_without_selectable_groups() {
    let mut pricing = pricing(json!(["openai"]));
    pricing["data"][0]["enable_groups"] = json!(["svip", "auto"]);
    assert!(Catalog::from_snapshots(&groups(), &pricing)
        .unwrap()
        .models
        .is_empty());
}

#[test]
fn wire_prefers_native_then_fixed_protocol_priority() {
    for (endpoints, fallback) in [
        (
            json!(["gemini", "anthropic", "openai", "openai-response"]),
            "openai-response",
        ),
        (json!(["gemini", "anthropic", "openai"]), "openai"),
        (json!(["gemini", "anthropic"]), "anthropic"),
        (json!(["gemini"]), "gemini"),
    ] {
        let pricing = pricing(endpoints.clone());
        let catalog = Catalog::from_snapshots(&groups(), &pricing).unwrap();
        for protocol in PROTOCOLS {
            let expected = if endpoints.as_array().unwrap().contains(&json!(protocol)) {
                protocol
            } else {
                fallback
            };
            assert_eq!(catalog.models["model-a"].wire[protocol], expected);
            assert_eq!(
                catalog.models["model-a"].compat[protocol],
                Compatibility::Unverified
            );
        }
    }
}

#[test]
fn unknown_endpoints_never_invent_a_wire() {
    for endpoints in [json!([]), json!(["future-protocol"])] {
        let catalog = Catalog::from_snapshots(&groups(), &pricing(endpoints)).unwrap();
        assert!(catalog.models["model-a"].wire.is_empty());
    }
}

#[test]
fn compatibility_overrides_apply_only_to_exact_protocol_and_model() {
    for (value, expected) in [
        ("full", Compatibility::Full),
        ("partial", Compatibility::Partial),
        ("text_only", Compatibility::TextOnly),
        ("unavailable", Compatibility::Unavailable),
    ] {
        let overrides =
            json!({"anthropic|model-a":{"wire":"openai-response","compat":value}}).to_string();
        let catalog =
            Catalog::build(&groups(), &pricing(json!(["anthropic"])), &overrides).unwrap();
        assert_eq!(
            catalog.models["model-a"].wire["anthropic"],
            "openai-response"
        );
        assert_eq!(catalog.models["model-a"].compat["anthropic"], expected);
        assert_eq!(
            catalog.models["model-a"].compat["openai"],
            Compatibility::Unverified
        );
    }
    assert!(
        serde_json::from_str::<BTreeMap<String, Override>>(include_str!("../compat.json"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn invalid_snapshots_or_override_configuration_fail_explicitly() {
    let good = pricing(json!(["openai"]));
    for overrides in [
        "not json",
        r#"{"wrong|model-a":{}}"#,
        r#"{"openai|model-a":{"wire":"invalid"}}"#,
        r#"{"openai|model-a":{"compat":"invalid"}}"#,
        r#"{"openai|model-a":{"typo":true}}"#,
    ] {
        assert_eq!(
            Catalog::build(&groups(), &good, overrides),
            Err(NewApiError::InvalidResponse)
        );
    }
    assert_eq!(
        Catalog::from_snapshots(&json!({}), &good),
        Err(NewApiError::InvalidResponse)
    );
    let mut duplicate = good.clone();
    duplicate["data"]
        .as_array_mut()
        .unwrap()
        .push(good["data"][0].clone());
    assert_eq!(
        Catalog::from_snapshots(&groups(), &duplicate),
        Err(NewApiError::InvalidResponse)
    );
}

#[test]
fn reads_only_persisted_snapshots_and_preserves_missing_cache() {
    use super::super::cache::{CacheEvents, CacheRemote};
    use futures::future::BoxFuture;
    use std::sync::Arc;

    struct NoNetwork;
    impl CacheRemote for NoNetwork {
        fn fetch<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
            panic!("catalog must not request network");
        }
        fn fetch_usage<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<Value, NewApiError>> {
            panic!("catalog must not request usage");
        }
    }
    impl CacheEvents for NoNetwork {
        fn data_updated(&self, _: &str) {
            panic!("reading catalog must not publish an update");
        }
    }
    let db = Arc::new(crate::Database::memory().unwrap());
    let cache = AicliCache::with_remote(db.clone(), Arc::new(NoNetwork), Arc::new(NoNetwork));
    assert_eq!(Catalog::from_cache(&cache).unwrap(), None);
    let groups = groups();
    let pricing = pricing(json!(["openai"]));
    for (kind, value) in [("groups", &groups), ("pricing", &pricing)] {
        db.conn.lock().unwrap().execute(
            "INSERT INTO aicli_cache(kind,fingerprint,fetched_at,body_json) VALUES (?1,'test',1,?2)",
            rusqlite::params![kind, value.to_string()],
        ).unwrap();
        if kind == "groups" {
            assert_eq!(Catalog::from_cache(&cache).unwrap(), None);
        }
    }
    assert_eq!(
        Catalog::from_cache(&cache).unwrap(),
        Some(Catalog::from_snapshots(&groups, &pricing).unwrap())
    );
}
