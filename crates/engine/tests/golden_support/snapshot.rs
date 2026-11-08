//! The recorder's output shapes: `result_json`, `snapshot` (with `candidate_json`), `dump_journal` and `query`. Optional fields are emitted only when the recorder emits them.

use std::path::Path;

use msime_engine::{KeyResult, Session, SessionSnapshot, WordItem};
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Map, Value};

/// `{handled, commit?, diagnostic?}` with diagnostics `$ROOT`-scrubbed.
pub fn result_json(result: &KeyResult, roots: &[String]) -> Value {
    let mut out = Map::new();
    out.insert("handled".into(), Value::from(result.handled));
    if let Some(commit) = &result.commit {
        out.insert("commit".into(), Value::from(commit.as_str()));
    }
    if let Some(diagnostic) = &result.diagnostic {
        out.insert("diagnostic".into(), Value::from(scrub(diagnostic, roots)));
    }
    Value::Object(out)
}

/// Every `SessionSnapshot` field, `segment_boundaries`, every candidate and the online query without its per-process `generation`, `identity` and `session_id`. `page` is `(index, size)` when the scenario pages.
pub fn snapshot_json(session: &Session, page: Option<(usize, usize)>) -> Value {
    let view = session.snapshot();
    let mut out = Map::new();
    out.insert("scheme".into(), Value::from(view.scheme.name()));
    out.insert("local_mode".into(), Value::from(view.local_mode.name()));
    out.insert("preedit".into(), Value::from(view.preedit.as_str()));
    out.insert(
        "raw_segmentation".into(),
        Value::from(view.raw_segmentation.as_str()),
    );
    out.insert(
        "normalized_segmentation".into(),
        Value::from(view.normalized_segmentation.as_str()),
    );
    out.insert(
        "editing_text".into(),
        Value::from(view.editing_text.as_str()),
    );
    out.insert("caret".into(), Value::from(view.caret_position));
    out.insert(
        "dedicated_english".into(),
        Value::from(view.dedicated_english),
    );
    out.insert(
        "shuangpin_profile".into(),
        Value::from(view.shuangpin_profile.as_str()),
    );
    let boundaries = session.segment_raw_boundaries();
    if !boundaries.is_empty() {
        out.insert("segment_boundaries".into(), Value::from(boundaries));
    }
    out.insert("candidate_count".into(), Value::from(view.candidates.len()));
    let candidates = view
        .candidates
        .iter()
        .enumerate()
        .map(|(index, item)| candidate_json(item, &view, index))
        .collect();
    out.insert("candidates".into(), Value::Array(candidates));
    if let Some((index, size)) = page {
        out.insert("page".into(), json!({"index": index, "size": size}));
    }
    if let Some(query) = session.online_query() {
        out.insert(
            "online_query".into(),
            json!({
                "scheme": query.scheme.name(),
                "query_text": query.query_text,
                "cache_key": query.cache_key,
                "pinyin_segments": query.pinyin_segments,
                "cloud_eligible": query.cloud_eligible,
                "ai_eligible": query.ai_eligible,
            }),
        );
    }
    Value::Object(out)
}

/// One candidate as `candidate_json` writes it: the flags and lists only when set, the annotation only when non-empty.
fn candidate_json(item: &WordItem, view: &SessionSnapshot, index: usize) -> Value {
    let mut out = Map::new();
    out.insert("word".into(), Value::from(item.word.as_str()));
    out.insert("pinyin".into(), Value::from(item.pinyin.as_str()));
    out.insert(
        "canonical_pinyin".into(),
        Value::from(item.canonical_pinyin.as_str()),
    );
    out.insert("source".into(), Value::from(item.source as u8));
    out.insert("scheme".into(), Value::from(item.scheme.name()));
    out.insert("weight".into(), Value::from(item.weight));
    if item.fixed_position != 0 {
        out.insert("fixed_position".into(), Value::from(item.fixed_position));
    }
    if item.fuzzy {
        out.insert("fuzzy".into(), Value::from(true));
    }
    if !item.corrected_from.is_empty() {
        out.insert(
            "corrected_from".into(),
            Value::from(item.corrected_from.as_str()),
        );
    }
    if item.sentence_association {
        out.insert("sentence_association".into(), Value::from(true));
    }
    if !item.sentence_words.is_empty() {
        out.insert(
            "sentence_words".into(),
            Value::from(item.sentence_words.clone()),
        );
    }
    if let Some(annotation) = view.candidate_annotations.get(index) {
        if !annotation.is_empty() {
            out.insert("annotation".into(), Value::from(annotation.as_str()));
        }
    }
    if let Some(answers_key) = view.candidate_answers_key.get(index) {
        out.insert("answers_key".into(), Value::from(*answers_key));
    }
    Value::Object(out)
}

/// Every journal table, every column but `updated_at` and `created_at`, ordered by all columns; flushes personal learning first.
pub fn dump_journal(journal: &Path) -> Value {
    msime_engine::flush_personal_learning();
    journal_tables(journal)
}

/// The table walk of `dump_journal` without the flush: `{table: {columns, rows}}`, leaving out tables with no rows or no kept column.
pub fn journal_tables(journal: &Path) -> Value {
    let mut result = Map::new();
    if !journal.exists() {
        return Value::Object(result);
    }
    let tables = query_rows(
        journal,
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    );
    for table in string_column(&tables) {
        let info = query_rows(journal, &format!("PRAGMA table_info(\"{table}\")"));
        let columns: Vec<String> = info
            .as_array()
            .expect("PRAGMA table_info returns rows")
            .iter()
            .filter_map(|row| row[1].as_str())
            .filter(|name| *name != "updated_at" && *name != "created_at")
            .map(str::to_owned)
            .collect();
        if columns.is_empty() {
            continue;
        }
        let select = columns
            .iter()
            .map(|column| format!("\"{column}\""))
            .collect::<Vec<_>>()
            .join(",");
        let order = (1..=columns.len())
            .map(|position| position.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let rows = query_rows(
            journal,
            &format!("SELECT {select} FROM \"{table}\" ORDER BY {order}"),
        );
        if rows.as_array().is_some_and(Vec::is_empty) {
            continue;
        }
        result.insert(table, json!({"columns": columns, "rows": rows}));
    }
    Value::Object(result)
}

fn string_column(rows: &Value) -> Vec<String> {
    rows.as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row.get(0).and_then(Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Read-only rows as arrays; `{"missing": true}` for an absent file and `{"error": ...}` for bad SQL.
pub fn query_rows(path: &Path, sql: &str) -> Value {
    if !path.exists() {
        return json!({"missing": true});
    }
    let Ok(connection) = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY) else {
        return json!({"error": "open failed"});
    };
    let mut statement = match connection.prepare(sql) {
        Ok(statement) => statement,
        Err(error) => return json!({"error": sqlite_message(&error)}),
    };
    let column_count = statement.column_count();
    let mut rows = Vec::new();
    let mut cursor = statement.raw_query();
    // The recorder stops at the first step that is not a row, so a runtime error ends the list rather than replacing it.
    while let Ok(Some(row)) = cursor.next() {
        let values = (0..column_count)
            .map(|column| column_value(row.get_ref_unwrap(column)))
            .collect();
        rows.push(Value::Array(values));
    }
    Value::Array(rows)
}

/// `sqlite3_column_*` by storage class, as the recorder reads them; a blob is read through `sqlite3_column_text`.
fn column_value(value: ValueRef<'_>) -> Value {
    match value {
        ValueRef::Integer(number) => Value::from(number),
        ValueRef::Real(number) => Value::from(number),
        ValueRef::Null => Value::Null,
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => {
            Value::from(String::from_utf8_lossy(bytes).into_owned())
        }
    }
}

/// `sqlite3_errmsg` text: the message SQLite attached, or its generic description.
fn sqlite_message(error: &rusqlite::Error) -> String {
    match error {
        rusqlite::Error::SqliteFailure(_, Some(message))
        | rusqlite::Error::SqlInputError { msg: message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// Replace every root spelling in a diagnostic with `$ROOT`, longest first.
pub fn scrub(text: &str, roots: &[String]) -> String {
    let mut ordered: Vec<&String> = roots.iter().filter(|root| !root.is_empty()).collect();
    ordered.sort_by_key(|root| std::cmp::Reverse(root.len()));
    let mut text = text.to_owned();
    for root in ordered {
        text = text.replace(root.as_str(), "$ROOT");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_replaces_the_canonical_spelling_before_its_alias() {
        let roots = vec!["/var/x".to_owned(), "/private/var/x".to_owned()];
        assert_eq!(
            scrub("/private/var/x/user/a.db and /var/x/cache", &roots),
            "$ROOT/user/a.db and $ROOT/cache"
        );
        assert_eq!(scrub("no path here", &roots), "no path here");
    }

    #[test]
    fn result_json_emits_optional_fields_only_when_set() {
        assert_eq!(
            result_json(&KeyResult::handled(), &[]),
            json!({"handled": true})
        );
        let result = KeyResult::committed("你好").with_diagnostic(Some("cannot open /r/x".into()));
        assert_eq!(
            result_json(&result, &["/r".to_owned()]),
            json!({"handled": true, "commit": "你好", "diagnostic": "cannot open $ROOT/x"})
        );
    }

    #[test]
    fn query_rows_keeps_storage_classes_and_reports_missing_and_bad_sql() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("q.db");
        assert_eq!(query_rows(&path, "SELECT 1"), json!({"missing": true}));
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch("CREATE TABLE t(a, b, c, d); INSERT INTO t VALUES(7, 1.5, NULL, '字');")
            .unwrap();
        drop(connection);
        assert_eq!(
            query_rows(&path, "SELECT a,b,c,d FROM t"),
            json!([[7, 1.5, null, "字"]])
        );
        let error = query_rows(&path, "SELECT nope FROM t");
        assert_eq!(error["error"], json!("no such column: nope"));
    }

    #[test]
    fn journal_tables_skip_timestamps_and_empty_tables_and_order_by_every_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("msime_user.db");
        assert_eq!(journal_tables(&path), json!({}));
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE ops(dictionary TEXT, key TEXT, weight INTEGER, updated_at INTEGER, created_at INTEGER);
                 INSERT INTO ops VALUES('pinyin','ni''hao',5,111,222);
                 INSERT INTO ops VALUES('english','hi',9,333,444);
                 INSERT INTO ops VALUES('pinyin','ni''hao',3,555,666);
                 CREATE TABLE empty(x);
                 CREATE TABLE stamps(updated_at INTEGER);
                 INSERT INTO stamps VALUES(1);",
            )
            .unwrap();
        drop(connection);
        assert_eq!(
            journal_tables(&path),
            json!({"ops": {
                "columns": ["dictionary", "key", "weight"],
                "rows": [["english", "hi", 9], ["pinyin", "ni'hao", 3], ["pinyin", "ni'hao", 5]],
            }})
        );
    }
}
