use github_actions_local_cache::model::EntryMetadata;
use serde_json::json;

fn valid_metadata() -> serde_json::Value {
    json!({
        "schema": 1,
        "archivePolicy": 1,
        "key": "key",
        "createdAt": "2026-08-28T00:00:00Z",
        "payloadBytes": 10,
        "payloadSha256": "a".repeat(64),
        "os": "linux",
        "arch": "x64",
        "paths": ["file"]
    })
}

#[test]
fn metadata_json_has_a_strict_shape() {
    let parsed: EntryMetadata = serde_json::from_value(valid_metadata()).unwrap();
    assert_eq!(parsed.key, "key");

    let mut unknown = valid_metadata();
    unknown["surprise"] = json!(true);
    assert!(serde_json::from_value::<EntryMetadata>(unknown).is_err());

    let mut wrong_type = valid_metadata();
    wrong_type["payloadBytes"] = json!("10");
    assert!(serde_json::from_value::<EntryMetadata>(wrong_type).is_err());
}
