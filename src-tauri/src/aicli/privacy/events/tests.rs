use super::*;
use crate::aicli::privacy::placeholder::PlaceholderEngine;
use crate::aicli::privacy::rules::known::shared_index;
use crate::aicli::privacy::rules::redact::{mask, MaskOptions};

// Same shapes the redact and e2e tests prove are masked (assembled so secret
// scanners do not flag the fixture).
fn secret() -> String {
    ["sk-", "Zx9QvLtYb3Nc8UmKe4Hd"].concat()
}

fn email() -> String {
    ["canary", "@", "mail", ".", "acme", "-", "corp", ".", "cn"].concat()
}

fn masked_text() -> String {
    masked_with_count().0
}

fn masked_with_count() -> (String, usize) {
    let mut engine = PlaceholderEngine::with_key([7u8; 32]);
    let options = MaskOptions {
        secrets: true,
        personal: true,
        ..Default::default()
    };
    let (text, count) = mask(
        &format!("first {} again {} mail {}", secret(), secret(), email()),
        &options,
        &mut engine,
    );
    assert!(count >= 3, "fixture must mask the key twice and the email");
    (text, count)
}

fn event(ts_ms: i64, app: &str, outcome: Outcome, masked: Option<String>) -> PendingEvent {
    PendingEvent {
        ts_ms,
        app_type: Some(app.to_owned()),
        line_id: Some("main".to_owned()),
        outcome,
        reason: None,
        masked: masked.map(bytes::Bytes::from),
    }
}

fn noon(day: NaiveDate) -> i64 {
    day_start_ms(day) + 12 * 60 * 60 * 1000
}

#[test]
fn scan_counts_kinds_and_distinct_values() {
    let (text, count) = masked_with_count();
    let scan = scan_kinds(&text);
    assert_eq!(scan.values, count as u64);
    // the key appears twice under one placeholder
    assert!(scan.distinct < scan.values);
    assert_eq!(scan.ids.len() as u64, scan.distinct);
    assert_eq!(scan.kinds.values().sum::<u64>(), scan.values);
    assert_eq!(scan.kinds.get("EMAIL"), Some(&1));
    // ids are 16 hex characters, never the placeholder itself
    assert!(scan
        .ids
        .iter()
        .all(|id| id.len() == 16 && id.chars().all(|c| c.is_ascii_hexdigit())));
}

#[test]
fn categories_group_kinds_and_default_unknown_to_custom() {
    assert_eq!(category_of("API_KEY"), "secret");
    assert_eq!(category_of("EMAIL"), "personal");
    assert_eq!(category_of("HOSTNAME"), "network");
    assert_eq!(category_of("TERM"), "custom");
    assert_eq!(category_of("MY_RULE"), "custom");
}

#[test]
fn records_hold_counts_and_hashes_but_never_values_or_placeholders() {
    let db = Database::memory().unwrap();
    let text = masked_text();
    let today = today();
    assert!(record_blocking(
        &db,
        &event(noon(today), "claude", Outcome::Masked, Some(text.clone()))
    )
    .unwrap());
    let conn = db.conn.lock().unwrap();
    let dump: Vec<String> = conn
        .prepare("SELECT * FROM aicli_privacy_event")
        .unwrap()
        .query_map([], |row| {
            Ok((0..11)
                .map(|i| {
                    row.get::<_, rusqlite::types::Value>(i)
                        .map(|value| format!("{value:?}"))
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>()
                .join("|"))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let dump = dump.join("\n");
    assert!(!dump.contains(&secret()), "{dump}");
    assert!(!dump.contains(&email()), "{dump}");
    assert!(!dump.contains("canary"), "{dump}");
    for placeholder in placeholder_regex().find_iter(&text) {
        assert!(!dump.contains(placeholder.as_str()), "{dump}");
    }
}

#[test]
fn masked_requests_without_placeholders_are_not_recorded() {
    let db = Database::memory().unwrap();
    let ts = noon(today());
    assert!(!record_blocking(
        &db,
        &event(ts, "codex", Outcome::Masked, Some("plain".into()))
    )
    .unwrap());
    // blocked and degraded requests are recorded even with nothing masked
    assert!(record_blocking(&db, &event(ts, "codex", Outcome::Blocked, None)).unwrap());
    assert!(record_blocking(&db, &event(ts, "codex", Outcome::Degraded, None)).unwrap());
    let conn = db.conn.lock().unwrap();
    let rows: i64 = conn
        .query_row("SELECT COUNT(*) FROM aicli_privacy_event", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(rows, 2);
}

#[test]
fn summary_totals_days_categories_apps_and_distinct_values() {
    let db = Database::memory().unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 3, 10).unwrap();
    let text = masked_text();
    let conn = db.conn.lock().unwrap();
    insert(
        &conn,
        &event(noon(today), "claude", Outcome::Masked, Some(text.clone())),
    )
    .unwrap();
    insert(
        &conn,
        &event(
            noon(today - Duration::days(1)),
            "codex",
            Outcome::Masked,
            Some(text.clone()),
        ),
    )
    .unwrap();
    insert(&conn, &event(noon(today), "codex", Outcome::Blocked, None)).unwrap();
    // outside a 7-day window
    insert(
        &conn,
        &event(
            noon(today - Duration::days(9)),
            "claude",
            Outcome::Masked,
            Some(text),
        ),
    )
    .unwrap();

    let per = scan_kinds(&masked_text());
    let week = summary(&conn, 7, today).unwrap();
    assert_eq!(week.days.len(), 7);
    assert_eq!(week.days.last().unwrap().day, "2026-03-10");
    assert_eq!(week.days.last().unwrap().values, per.values);
    assert_eq!(week.days[5].values, per.values);
    assert_eq!(week.total_values, 2 * per.values);
    assert_eq!(week.requests, 2);
    // the same values were masked in both requests
    assert_eq!(week.distinct_values, per.distinct);
    assert_eq!(week.blocked, 1);
    let personal = week
        .categories
        .iter()
        .find(|c| c.key == "personal")
        .unwrap();
    assert_eq!(personal.count, 2);
    assert_eq!(
        week.apps.iter().map(|a| a.count).sum::<u64>(),
        2 * per.values
    );
    assert_eq!(week.retention_days, RETENTION_DAYS);

    let month = summary(&conn, 30, today).unwrap();
    assert_eq!(month.days.len(), 30);
    assert_eq!(month.total_values, 3 * per.values);
    // a range above the retention is clamped
    assert_eq!(
        summary(&conn, 400, today).unwrap().range_days,
        RETENTION_DAYS as u32
    );
}

#[test]
fn list_pages_newest_first_and_filters_by_category() {
    let db = Database::memory().unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 3, 10).unwrap();
    let conn = db.conn.lock().unwrap();
    let text = masked_text();
    for minute in 0..5 {
        insert(
            &conn,
            &event(
                noon(today) + minute * 60_000,
                "claude",
                Outcome::Masked,
                Some(text.clone()),
            ),
        )
        .unwrap();
    }
    insert(&conn, &event(noon(today), "codex", Outcome::Blocked, None)).unwrap();

    let first = list(&conn, 7, today, None, None, 4).unwrap();
    assert_eq!(first.items.len(), 4);
    assert!(first.items.windows(2).all(|w| w[0].id > w[1].id));
    let rest = list(&conn, 7, today, None, first.next_before_id, 4).unwrap();
    assert_eq!(rest.items.len(), 2);
    assert_eq!(rest.next_before_id, None);

    let personal = list(&conn, 7, today, Some("personal"), None, 100).unwrap();
    assert_eq!(personal.items.len(), 5);
    assert!(personal.items.iter().all(|item| item.outcome == "masked"));
    let network = list(&conn, 7, today, Some("network"), None, 100).unwrap();
    assert!(network.items.is_empty());
}

#[test]
fn prune_drops_records_past_retention_and_clear_drops_all() {
    let db = Database::memory().unwrap();
    let now = now_ms();
    let conn = db.conn.lock().unwrap();
    let old = now - (RETENTION_DAYS + 1) * 24 * 60 * 60 * 1000;
    insert(&conn, &event(old, "claude", Outcome::Blocked, None)).unwrap();
    insert(&conn, &event(now, "claude", Outcome::Blocked, None)).unwrap();
    assert_eq!(prune(&conn, now).unwrap(), 1);
    assert_eq!(clear(&conn).unwrap(), 1);
}

#[test]
fn ephemeral_engine_leaves_the_shared_known_index_untouched() {
    let mut engine = PlaceholderEngine::ephemeral();
    let options = MaskOptions {
        secrets: true,
        personal: true,
        ..Default::default()
    };
    let (text, count) = mask(&format!("token {}", secret()), &options, &mut engine);
    assert_eq!(count, 1);
    assert!(!text.contains(&secret()));
    assert!(shared_index().lock().unwrap().is_empty());

    // a production engine does register the value
    let mut production = PlaceholderEngine::with_key([1u8; 32]);
    mask(&format!("token {}", secret()), &options, &mut production);
    assert!(!shared_index().lock().unwrap().is_empty());
}
