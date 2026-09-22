use crate::error::{FrameworkError, Result};
use crate::jev::JevSnapshotUpdate;
use serde_json::{Map, Value};

pub fn apply_snapshot_update(snapshot: &mut Value, update: &JevSnapshotUpdate) -> Result<()> {
    if snapshot.is_null() {
        *snapshot = Value::Object(Map::new());
    }
    let object = snapshot.as_object_mut().ok_or_else(|| {
        FrameworkError::InvalidData("Jev snapshot root must be a JSON object".to_string())
    })?;
    for key in &update.remove {
        validate_top_level_key(key)?;
        object.remove(key);
    }
    for (key, value) in &update.set {
        validate_top_level_key(key)?;
        object.insert(key.clone(), value.clone());
    }
    Ok(())
}

fn validate_top_level_key(key: &str) -> Result<()> {
    if key.trim().is_empty() || key.contains('/') {
        return Err(FrameworkError::ValidationError(format!(
            "Jev snapshot update key '{key}' must be a non-empty top-level field"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[test]
    fn update_can_replace_and_remove_fields() {
        let mut snapshot = serde_json::json!({"regex":"old", "matches":["stale"]});
        let update = JevSnapshotUpdate {
            set: BTreeMap::from([("regex".to_string(), serde_json::json!("new"))]),
            remove: vec!["matches".to_string()],
        };
        apply_snapshot_update(&mut snapshot, &update).unwrap();
        assert_eq!(snapshot, serde_json::json!({"regex":"new"}));
    }
}
