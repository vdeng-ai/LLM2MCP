use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug)]
pub struct Snapshot {
    pub path: String,
    pub original: String,
    pub hash: String,
    pub visible: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    pub path: String,
    pub original_sha256: String,
    pub old_text: String,
    pub new_text: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NewDocument {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Updates {
    pub edits: Vec<Edit>,
    #[serde(default)]
    pub new_documents: Vec<NewDocument>,
}

pub fn validate(
    value: serde_json::Value,
    snapshots: &[Snapshot],
    root: &std::path::Path,
) -> Result<Updates> {
    let updates: Updates =
        serde_json::from_value(value).context("invalid document edits schema")?;
    if updates.edits.len() > 128 || updates.new_documents.len() > 8 {
        bail!("document update exceeds edit count limit");
    }
    let mut occupied: HashMap<&str, Vec<(usize, usize)>> = HashMap::new();
    for edit in &updates.edits {
        let document = snapshots
            .iter()
            .find(|document| document.path == edit.path)
            .context("document edit references a path not supplied to the model")?;
        if edit.original_sha256 != document.hash {
            bail!("document edit has an incorrect source hash: {}", edit.path);
        }
        if edit.old_text.is_empty() || !document.visible.contains(&edit.old_text) {
            bail!(
                "document edit must reference a nonempty supplied source fragment: {}",
                edit.path
            );
        }
        let matches = document
            .original
            .match_indices(&edit.old_text)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            bail!(
                "document edit source must match exactly once: {}",
                edit.path
            );
        }
        let start = matches[0].0;
        let end = start + edit.old_text.len();
        let ranges = occupied.entry(&edit.path).or_default();
        if ranges
            .iter()
            .any(|&(left, right)| start < right && left < end)
        {
            bail!("overlapping document edits: {}", edit.path);
        }
        ranges.push((start, end));
        if crate::privacy::redact(&edit.new_text) != edit.new_text {
            bail!("document edit contains sensitive replacement text");
        }
    }
    let mut new_paths = std::collections::HashSet::new();
    for document in &updates.new_documents {
        let path = std::path::Path::new(&document.path);
        if !document.path.starts_with("docs/")
            || path.extension().and_then(|ext| ext.to_str()) != Some("md")
            || path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
            || document.path.contains('\\')
            || !new_paths.insert(&document.path)
        {
            bail!("new document must have a unique relative docs/*.md path");
        }
        let mut ancestor = root.join(path);
        while !ancestor.exists() {
            ancestor = ancestor
                .parent()
                .context("invalid new document path")?
                .to_path_buf();
        }
        if root.join(path).exists() || !ancestor.canonicalize()?.starts_with(root.canonicalize()?) {
            bail!("new document already exists or escapes the workspace");
        }
        if document.content.trim().is_empty()
            || crate::privacy::redact(&document.content) != document.content
        {
            bail!("new document content must be nonempty and free of detected credentials");
        }
    }
    Ok(updates)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Snapshot {
        let original = "# Intro\nOld behavior\n\n# Unseen appendix\nKEEP THIS SECTION\n".to_owned();
        Snapshot {
            hash: crate::repo_cache::hash_bytes(original.as_bytes()),
            path: "README.md".into(),
            original,
            visible: "# Intro\nOld behavior\n".into(),
        }
    }
    fn update(snapshot: &Snapshot, old: &str) -> serde_json::Value {
        serde_json::json!({"edits":[{"path":snapshot.path,"original_sha256":snapshot.hash,"old_text":old,"new_text":"New behavior"}]})
    }
    #[test]
    fn local_edits_preserve_unseen_original_sections() {
        let snapshot = snapshot();
        let root = tempfile::tempdir().unwrap();
        let edits = validate(
            update(&snapshot, "Old behavior"),
            std::slice::from_ref(&snapshot),
            root.path(),
        )
        .unwrap();
        let changed =
            snapshot
                .original
                .replacen(&edits.edits[0].old_text, &edits.edits[0].new_text, 1);
        assert!(changed.contains("# Unseen appendix\nKEEP THIS SECTION"));
        assert!(changed.contains("New behavior"));
    }
    #[test]
    fn rejects_unseen_ambiguous_stale_and_overlapping_edits() {
        let mut snapshot = snapshot();
        let root = tempfile::tempdir().unwrap();
        assert!(
            validate(
                update(&snapshot, "KEEP THIS SECTION"),
                std::slice::from_ref(&snapshot),
                root.path()
            )
            .is_err()
        );
        let mut value = update(&snapshot, "Old behavior");
        value["edits"][0]["original_sha256"] = serde_json::json!("stale");
        assert!(validate(value, std::slice::from_ref(&snapshot), root.path()).is_err());
        let mut value = update(&snapshot, "Old behavior");
        let duplicate = value["edits"][0].clone();
        value["edits"].as_array_mut().unwrap().push(duplicate);
        assert!(validate(value, std::slice::from_ref(&snapshot), root.path()).is_err());
        snapshot.original.push_str("Old behavior");
        assert!(validate(update(&snapshot, "Old behavior"), &[snapshot], root.path()).is_err());
    }
}
