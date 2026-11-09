use super::*;
use serde_json::json;

fn catalog() -> Catalog {
    Catalog::from_snapshots(
        &json!({"data":{"cheap":{"ratio":0.5},"mid":{"ratio":1},"unknown":{},"auto":{}}}),
        &json!({"data":[
            {"model_name":"shared","enable_groups":["cheap","mid","unknown"],"supported_endpoint_types":["openai"]},
            {"model_name":"unique","enable_groups":["mid"],"supported_endpoint_types":["anthropic"]}
        ],"auto_groups":["mid","cheap","unknown"]}),
    ).unwrap()
}

fn user() -> Profile {
    let mut profile = preset("recommended", &catalog()).unwrap();
    profile.id = "p1".into();
    profile.name = "My profile".into();
    profile.source = "user".into();
    profile.preset_id = None;
    profile.body.resolved_conflicts = vec!["shared".into()];
    profile
}

#[test]
fn presets_remain_unpersisted_read_only_and_default_is_recommended() {
    let db = Arc::new(Database::memory().unwrap());
    let store = ProfileStore::new(db);
    let catalog = catalog();
    let recommended = preset("recommended", &catalog).unwrap();
    assert_eq!(
        recommended.body.selected_groups,
        ["mid", "cheap", "unknown"]
    );
    assert_eq!(effective_profile(None, &catalog).unwrap(), recommended);
    assert_eq!(effective_profile(Some(&user()), &catalog).unwrap(), user());
    assert_eq!(
        preset("low_cost", &catalog).unwrap().body.selected_groups,
        ["cheap", "mid", "unknown"]
    );
    assert_eq!(
        store.save(&recommended, &catalog),
        Err(ProfileError::ReadOnly)
    );
    assert!(store.list().unwrap().is_empty());
    assert!(preset("unsupported", &catalog).is_err());
    assert_eq!(
        compile(&recommended, &catalog, 3).unwrap()[0].ordered_groups,
        ["mid", "cheap", "unknown"]
    );
}

#[test]
fn unresolved_conflicts_are_reported_as_an_exact_list() {
    let mut profile = user();
    profile.body.resolved_conflicts.clear();
    assert_eq!(
        validate(&profile, &catalog()),
        Err(ProfileError::UnresolvedConflicts(vec!["shared".into()]))
    );
    profile.body.model_bindings.push(ModelBinding {
        model: "shared".into(),
        source_group: Some("cheap".into()),
        alias: None,
    });
    assert_eq!(validate(&profile, &catalog()), Ok(()));
}

#[test]
fn fixed_and_cost_first_orders_are_stable_and_unknown_ratio_is_last() {
    let mut profile = user();
    let catalog = catalog();
    assert_eq!(
        compile(&profile, &catalog, 3).unwrap()[0].ordered_groups,
        ["mid", "cheap", "unknown"]
    );
    profile.body.selection_mode = SelectionMode::CostFirst;
    assert_eq!(
        compile(&profile, &catalog, 3).unwrap()[0].ordered_groups,
        ["cheap", "mid", "unknown"]
    );
    let mut tied = catalog.clone();
    tied.groups.get_mut("cheap").unwrap().ratio = Some(1.0);
    assert_eq!(
        compile(&profile, &tied, 3).unwrap()[0].ordered_groups,
        ["mid", "cheap", "unknown"]
    );
    tied.groups.get_mut("cheap").unwrap().ratio_known = false;
    assert_eq!(
        compile(&profile, &tied, 3).unwrap()[0].ordered_groups,
        ["mid", "cheap", "unknown"]
    );
}

#[test]
fn explicit_source_stays_first_and_fallback_can_be_disabled() {
    let mut profile = user();
    profile.body.selection_mode = SelectionMode::CostFirst;
    profile.body.model_bindings.push(ModelBinding {
        model: "shared".into(),
        source_group: Some("mid".into()),
        alias: Some("renamed".into()),
    });
    let routes = compile(&profile, &catalog(), 3).unwrap();
    assert_eq!(
        routes[0],
        ModelRoute {
            public_model: "renamed".into(),
            upstream_model: "shared".into(),
            ordered_groups: vec!["mid".into(), "cheap".into(), "unknown".into()]
        }
    );
    profile.body.allow_fallback = false;
    assert_eq!(
        compile(&profile, &catalog(), 1).unwrap()[0].ordered_groups,
        ["mid"]
    );
}

#[test]
fn excessive_group_count_is_an_error_and_never_truncates() {
    let profile = user();
    assert_eq!(
        compile(&profile, &catalog(), 2),
        Err(ProfileError::TooManyGroups {
            model: "shared".into(),
            count: 3,
            max_count: 2
        })
    );
    assert_eq!(
        compile(&profile, &catalog(), 0),
        Err(ProfileError::TooManyGroups {
            model: "shared".into(),
            count: 3,
            max_count: 0
        })
    );
    assert_eq!(
        compile(&profile, &catalog(), 3).unwrap()[0]
            .ordered_groups
            .len(),
        3
    );
}

#[test]
fn alias_collisions_include_implicit_unbound_model_names() {
    let mut profile = user();
    profile.body.model_bindings.push(ModelBinding {
        model: "shared".into(),
        source_group: None,
        alias: Some("unique".into()),
    });
    assert_eq!(
        validate(&profile, &catalog()),
        Err(ProfileError::DuplicateAlias)
    );
    profile.body.model_bindings[0].alias = Some("copy".into());
    profile.body.model_bindings.push(ModelBinding {
        model: "shared".into(),
        source_group: Some("cheap".into()),
        alias: Some("copy".into()),
    });
    assert_eq!(
        validate(&profile, &catalog()),
        Err(ProfileError::DuplicateAlias)
    );
    profile.body.model_bindings[1].alias = Some("copy-two".into());
    let routes = compile(&profile, &catalog(), 3).unwrap();
    assert_eq!(routes.len(), 3);
    assert_eq!(routes[0].upstream_model, "shared");
    assert_eq!(routes[1].upstream_model, "shared");
    assert_ne!(routes[0].public_model, routes[1].public_model);
}

#[test]
fn invalid_binding_model_source_empty_alias_and_duplicate_groups_are_rejected() {
    for binding in [
        ModelBinding {
            model: "missing".into(),
            source_group: None,
            alias: None,
        },
        ModelBinding {
            model: "unique".into(),
            source_group: Some("cheap".into()),
            alias: None,
        },
        ModelBinding {
            model: "shared".into(),
            source_group: None,
            alias: Some(" ".into()),
        },
    ] {
        let mut profile = user();
        profile.body.model_bindings.push(binding);
        assert_eq!(validate(&profile, &catalog()), Err(ProfileError::Invalid));
    }
    let mut profile = user();
    profile.body.selected_groups.push("mid".into());
    assert_eq!(validate(&profile, &catalog()), Err(ProfileError::Invalid));
    let mut profile = user();
    profile.body.selected_groups = vec!["mid".into()];
    profile.body.model_bindings.push(ModelBinding {
        model: "shared".into(),
        source_group: Some("cheap".into()),
        alias: None,
    });
    assert_eq!(validate(&profile, &catalog()), Err(ProfileError::Invalid));
}

#[test]
fn invalid_identity_revision_and_line_policy_are_rejected_without_writes() {
    let store = ProfileStore::new(Arc::new(Database::memory().unwrap()));
    for profile in [
        Profile {
            id: String::new(),
            ..user()
        },
        Profile {
            name: " ".into(),
            ..user()
        },
        Profile {
            revision: -1,
            ..user()
        },
        Profile {
            source: "unknown".into(),
            ..user()
        },
    ] {
        assert!(store.save(&profile, &catalog()).is_err());
    }
    let mut profile = user();
    profile.body.line_policy = "unknown".into();
    assert_eq!(store.save(&profile, &catalog()), Err(ProfileError::Invalid));
    for line in ["inherit", "main", "global", "apac"] {
        profile.body.line_policy = line.into();
        assert_eq!(validate(&profile, &catalog()), Ok(()));
    }
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn optimistic_revisions_preserve_data_on_stale_or_invalid_saves() {
    let db = Arc::new(Database::memory().unwrap());
    let store = ProfileStore::new(db.clone());
    let first = store.save(&user(), &catalog()).unwrap();
    assert_eq!(first.revision, 1);
    assert_eq!(
        store.save(&user(), &catalog()),
        Err(ProfileError::RevisionConflict)
    );
    let mut update = first.clone();
    update.name = "Updated".into();
    let second = store.save(&update, &catalog()).unwrap();
    assert_eq!(second.revision, 2);
    assert_eq!(
        store.save(&update, &catalog()),
        Err(ProfileError::RevisionConflict)
    );
    let mut bad = second.clone();
    bad.body.selected_groups.push("removed".into());
    assert_eq!(
        store.save(&bad, &catalog()),
        Err(ProfileError::MissingGroups(vec!["removed".into()]))
    );
    assert_eq!(ProfileStore::new(db).list().unwrap(), vec![second]);
}

#[test]
fn missing_groups_do_not_delete_or_mutate_persisted_profiles() {
    let store = ProfileStore::new(Arc::new(Database::memory().unwrap()));
    let profile = store.save(&user(), &catalog()).unwrap();
    let mut changed = catalog();
    changed.groups.remove("cheap");
    for model in changed.models.values_mut() {
        model.groups.retain(|group| group != "cheap");
    }
    let repair = needs_repair(&profile, &changed);
    assert!(repair.needs_repair);
    assert_eq!(repair.missing_groups, ["cheap"]);
    assert!(repair.affected_models.contains(&"shared".into()));
    assert_eq!(
        compile(&profile, &changed, 3),
        Err(ProfileError::NeedsRepair(repair))
    );
    assert_eq!(store.list().unwrap(), vec![profile]);
}

#[test]
fn sixty_nine_models_form_at_most_eight_ordered_group_buckets() {
    let clusters = [
        vec!["gpt", "gpt-max", "gpt-pro"],
        vec!["claude-aws", "claude-code", "claude-kiro"],
        vec!["gemini-cli"],
        vec!["xAI"],
    ];
    let mut groups = serde_json::Map::new();
    let mut rows = Vec::new();
    let mut selected = Vec::new();
    for cluster in &clusters {
        for (i, group) in cluster.iter().enumerate() {
            groups.insert((*group).into(), json!({"ratio":i as f64+0.5}));
            selected.push((*group).to_string());
        }
    }
    for i in 0..69 {
        let cluster = &clusters[i % clusters.len()];
        rows.push(json!({"model_name":format!("{}-fixture-{i:02}",cluster[0]),"enable_groups":cluster,"supported_endpoint_types":["openai"]}));
    }
    let catalog = Catalog::from_snapshots(
        &json!({"data":groups}),
        &json!({"data":rows,"auto_groups":selected}),
    )
    .unwrap();
    let mut profile = preset("recommended", &catalog).unwrap();
    for mode in [SelectionMode::FixedPriority, SelectionMode::CostFirst] {
        profile.body.selection_mode = mode;
        let routes = compile(&profile, &catalog, 15).unwrap();
        assert_eq!(routes.len(), 69);
        let buckets: BTreeSet<_> = routes
            .iter()
            .map(|route| route.ordered_groups.clone())
            .collect();
        assert_eq!(buckets.len(), 4);
        assert!(buckets.len() <= 8);
        for route in &routes {
            assert!(!route.ordered_groups.is_empty());
            assert!(route.ordered_groups.len() <= 3);
        }
    }
}

#[test]
fn fallback_never_uses_unselected_groups_and_source_must_be_selected_on_save() {
    let store = ProfileStore::new(Arc::new(Database::memory().unwrap()));
    let mut profile = user();
    profile.body.selected_groups = vec!["mid".into(), "cheap".into()];
    profile.body.model_bindings = vec![ModelBinding {
        model: "shared".into(),
        source_group: Some("mid".into()),
        alias: None,
    }];
    assert_eq!(
        compile(&profile, &catalog(), 3).unwrap()[0].ordered_groups,
        ["mid", "cheap"]
    );
    profile.body.selection_mode = SelectionMode::CostFirst;
    assert_eq!(
        compile(&profile, &catalog(), 3).unwrap()[0].ordered_groups,
        ["mid", "cheap"]
    );
    profile.body.model_bindings[0].source_group = Some("unknown".into());
    assert_eq!(store.save(&profile, &catalog()), Err(ProfileError::Invalid));
    assert!(store.list().unwrap().is_empty());
}

#[test]
fn copied_and_reset_presets_confirm_only_current_conflicts_and_keep_preset_routes() {
    let store = ProfileStore::new(Arc::new(Database::memory().unwrap()));
    let catalog = catalog();
    for id in ["recommended", "low_cost"] {
        let copied = store
            .copy_preset(id, &format!("copy-{id}"), "Copy", &catalog)
            .unwrap();
        assert_eq!(copied.source, "system_copy");
        assert_eq!(copied.body.resolved_conflicts, ["shared"]);
        assert!(copied.body.model_bindings.is_empty());
        assert_eq!(
            compile(&copied, &catalog, 3).unwrap(),
            compile(&preset(id, &catalog).unwrap(), &catalog, 3).unwrap()
        );
        let mut changed = copied.clone();
        changed.body.model_bindings.push(ModelBinding {
            model: "shared".into(),
            source_group: Some("cheap".into()),
            alias: Some("renamed".into()),
        });
        let changed = store.save(&changed, &catalog).unwrap();
        let reset = store.reset_to_preset(&changed, id, &catalog).unwrap();
        assert!(reset.body.model_bindings.is_empty());
        assert_eq!(reset.body.resolved_conflicts, ["shared"]);
        assert_eq!(
            compile(&reset, &catalog, 3).unwrap(),
            compile(&preset(id, &catalog).unwrap(), &catalog, 3).unwrap()
        );
    }
}

#[test]
fn new_catalog_conflict_does_not_block_compile_but_next_save_requires_confirmation() {
    let store = ProfileStore::new(Arc::new(Database::memory().unwrap()));
    let profile = store
        .copy_preset("recommended", "copy", "Copy", &catalog())
        .unwrap();
    let mut changed = catalog();
    let mut model = changed.models["shared"].clone();
    model.pricing["model_name"] = json!("new-conflict");
    changed.models.insert("new-conflict".into(), model);
    let compiled = store.compile(&profile, &changed, 3).unwrap();
    assert!(!compiled.repair.needs_repair);
    assert!(compiled
        .routes
        .iter()
        .any(|r| r.public_model == "new-conflict"));
    assert_eq!(
        store.save(&profile, &changed),
        Err(ProfileError::UnresolvedConflicts(vec![
            "new-conflict".into()
        ]))
    );
    assert!(!store.list().unwrap()[0]
        .body
        .resolved_conflicts
        .contains(&"new-conflict".into()));
}

#[test]
fn successful_compile_persists_only_cache_baseline_without_revision_or_body_changes() {
    let db = Arc::new(Database::memory().unwrap());
    let store = ProfileStore::new(db.clone());
    let profile = store.save(&user(), &catalog()).unwrap();
    let before = store.list().unwrap();
    let compiled = store.compile(&profile, &catalog(), 3).unwrap();
    let baseline = store.baseline(&profile.id).unwrap().unwrap();
    assert_eq!(baseline.routes["shared"], ["mid", "cheap", "unknown"]);
    assert!(baseline.compiled_at > 0);
    assert_eq!(baseline.fingerprint.len(), 64);
    assert_eq!(store.list().unwrap(), before);
    let cached:(String,String,i64)=db.conn.lock().unwrap().query_row("SELECT body_json,fingerprint,fetched_at FROM aicli_cache WHERE kind='profile_baseline:p1'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    let value: serde_json::Value = serde_json::from_str(&cached.0).unwrap();
    assert_eq!(value.as_object().unwrap().len(), 3);
    assert_eq!(cached.1, baseline.fingerprint);
    assert_eq!(cached.2, baseline.compiled_at);
    assert_eq!(compiled.routes.len(), 2);
    assert_eq!(
        ProfileStore::new(db).baseline("p1").unwrap(),
        Some(baseline)
    );
}

#[test]
fn baseline_identifies_disappeared_unique_models_and_revoked_fallbacks_without_overwrite() {
    let store = ProfileStore::new(Arc::new(Database::memory().unwrap()));
    let mut initial = catalog();
    let mut unique = initial.models["unique"].clone();
    unique.groups = vec!["cheap".into()];
    initial.models.insert("gone-unique".into(), unique);
    let profile = store.save(&user(), &initial).unwrap();
    store.compile(&profile, &initial, 3).unwrap();
    let baseline = store.baseline("p1").unwrap().unwrap();
    let mut changed = initial.clone();
    changed.groups.remove("cheap");
    changed.models.remove("gone-unique");
    for model in changed.models.values_mut() {
        model.groups.retain(|g| g != "cheap");
    }
    let result = store.compile(&profile, &changed, 3).unwrap();
    assert_eq!(result.repair.missing_groups, ["cheap"]);
    assert!(!result.repair.conservative);
    assert_eq!(result.repair.affected_models, ["gone-unique", "shared"]);
    assert_eq!(
        result
            .routes
            .iter()
            .map(|r| r.public_model.as_str())
            .collect::<Vec<_>>(),
        ["unique"]
    );
    assert_eq!(store.baseline("p1").unwrap(), Some(baseline));
    assert_eq!(store.list().unwrap(), vec![profile]);
}

#[test]
fn missing_explicit_source_is_affected_even_when_absent_from_baseline() {
    let mut profile = user();
    profile.body.model_bindings.push(ModelBinding {
        model: "old-model".into(),
        source_group: Some("removed".into()),
        alias: Some("old-alias".into()),
    });
    let baseline = ProfileBaseline {
        fingerprint: "test".into(),
        compiled_at: 1,
        routes: BTreeMap::new(),
    };
    let repair = repair_with_baseline(&profile, &catalog(), Some(&baseline));
    assert_eq!(repair.affected_models, ["old-alias"]);
    assert!(!repair.conservative);
}

#[test]
fn no_baseline_conservatively_disables_every_route_and_does_not_create_baseline() {
    let store = ProfileStore::new(Arc::new(Database::memory().unwrap()));
    let profile = store.save(&user(), &catalog()).unwrap();
    let mut changed = catalog();
    changed.groups.remove("cheap");
    let result = store.compile(&profile, &changed, 3).unwrap();
    assert!(result.repair.conservative);
    assert!(result.routes.is_empty());
    assert_eq!(result.repair.affected_models, ["shared", "unique"]);
    assert_eq!(store.baseline("p1").unwrap(), None);
    assert_eq!(store.list().unwrap(), vec![profile]);
}

#[test]
fn failed_compile_preserves_baseline_and_delete_removes_only_matching_baseline() {
    let db = Arc::new(Database::memory().unwrap());
    let store = ProfileStore::new(db.clone());
    let profile = store.save(&user(), &catalog()).unwrap();
    store.compile(&profile, &catalog(), 3).unwrap();
    let before = store.baseline("p1").unwrap();
    assert!(store.compile(&profile, &catalog(), 2).is_err());
    assert_eq!(store.baseline("p1").unwrap(), before);
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO aicli_cache VALUES ('profile_baseline:other','other',1,'{}')",
            [],
        )
        .unwrap();
    assert_eq!(store.delete("p1", 0), Err(ProfileError::RevisionConflict));
    assert_eq!(store.baseline("p1").unwrap(), before);
    store.delete("p1", profile.revision).unwrap();
    assert!(store.list().unwrap().is_empty());
    assert_eq!(store.baseline("p1").unwrap(), None);
    let count: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT count(*) FROM aicli_cache WHERE kind='profile_baseline:other'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(store.delete("recommended", 0), Err(ProfileError::ReadOnly));
}

#[test]
fn revoked_alias_source_does_not_publish_an_implicit_unaliased_route() {
    let store = ProfileStore::new(Arc::new(Database::memory().unwrap()));
    let mut profile = user();
    profile.body.model_bindings.push(ModelBinding {
        model: "shared".into(),
        source_group: Some("cheap".into()),
        alias: Some("custom".into()),
    });
    let profile = store.save(&profile, &catalog()).unwrap();
    store.compile(&profile, &catalog(), 3).unwrap();
    let mut changed = catalog();
    changed.groups.remove("cheap");
    for model in changed.models.values_mut() {
        model.groups.retain(|g| g != "cheap");
    }
    let compiled = store.compile(&profile, &changed, 3).unwrap();
    assert_eq!(compiled.repair.affected_models, ["custom"]);
    assert_eq!(
        compiled
            .routes
            .iter()
            .map(|r| r.public_model.as_str())
            .collect::<Vec<_>>(),
        ["unique"]
    );
}

#[test]
fn excluded_models_are_neither_routed_nor_conflicts() {
    let mut profile = user();
    profile.body.resolved_conflicts.clear();
    // 未处理的冲突模型被排除：不再报冲突，也不编译进路由
    profile.body.excluded_models = vec!["shared".into()];
    assert_eq!(validate(&profile, &catalog()), Ok(()));
    let routes = compile(&profile, &catalog(), 8).unwrap();
    assert!(routes.iter().all(|route| route.upstream_model != "shared"));
    assert!(routes.iter().any(|route| route.upstream_model == "unique"));

    // 排除又绑定同一模型：自相矛盾
    profile.body.model_bindings.push(ModelBinding {
        model: "shared".into(),
        source_group: Some("cheap".into()),
        alias: None,
    });
    assert_eq!(validate(&profile, &catalog()), Err(ProfileError::Invalid));

    // 目录里已不存在的排除项不影响校验
    let mut gone = user();
    gone.body.excluded_models = vec!["retired-model".into()];
    assert_eq!(validate(&gone, &catalog()), Ok(()));
}

#[test]
fn bodies_saved_before_exclusions_existed_still_parse() {
    let body: ProfileBody = serde_json::from_value(json!({
        "selected_groups": ["mid"], "selection_mode": "fixed_priority",
        "model_bindings": [], "resolved_conflicts": [], "line_policy": "inherit",
        "allow_fallback": true
    }))
    .unwrap();
    assert!(body.excluded_models.is_empty());
}
