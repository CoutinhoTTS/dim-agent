//! The model names one provider's catalog publishes. They are unique within
//! that provider and computed on every read, so nothing stores them.
use crate::model_catalog::{ModelEntry, ModelMetadata};
use std::collections::{HashMap, HashSet};

/// What a renamed row puts between its own name and its qualifier.
const SEPARATOR: &str = " · ";

/// The fields a listing states its per-row label in, in the order the lookup
/// reads them. Another gateway's label is added here.
const LABELS: &[&str] = &["magpie_label"];

/// Where a listing says one row comes from: the text after the last separator
/// of the label it states for that row, which is the row's name followed by its
/// origin. `None` when it states no label, or one that names no origin.
fn label_qualifier(metadata: &ModelMetadata) -> Option<&str> {
    LABELS.iter().find_map(|label| {
        let value = metadata.raw.get(*label)?.as_str()?;
        let (_, qualifier) = value.rsplit_once(SEPARATOR)?;
        Some(qualifier.trim()).filter(|qualifier| !qualifier.is_empty())
    })
}

/// The name each of one provider's complete rows publishes, in their order,
/// each unique within that provider. Every row takes part — a disabled or an
/// unavailable one too — so switching a row on or off renames no other, and
/// every client reads the same names.
///
/// A name compares after trimming. A row whose name no other row states keeps
/// it as stated. Every row that states a shared name publishes `<its own name> ·
/// <qualifier>`: the origin its gateway states for it, else its id. A qualified
/// name another row publishes falls back to the id, and an id-qualified name
/// that is taken too gains a counter, so any input reaches unique names.
pub(crate) fn unique<'a>(entries: impl IntoIterator<Item = &'a ModelEntry>) -> Vec<String> {
    let entries: Vec<&ModelEntry> = entries.into_iter().collect();
    let mut stated: HashMap<&str, usize> = HashMap::new();
    for entry in &entries {
        *stated.entry(entry.display_name.trim()).or_default() += 1;
    }
    // A name one entry states publishes as it stands, so every qualified name
    // avoids it: the answer names an entry by its own name wherever it can.
    let mut taken: HashSet<String> = entries
        .iter()
        .filter(|entry| stated[entry.display_name.trim()] == 1)
        .map(|entry| entry.display_name.clone())
        .collect();
    let mut names = Vec::with_capacity(entries.len());
    for entry in entries {
        if stated[entry.display_name.trim()] == 1 {
            names.push(entry.display_name.clone());
            continue;
        }
        let name = qualify(entry, &taken);
        taken.insert(name.clone());
        names.push(name);
    }
    names
}

/// The name one entry publishes when another entry states its name too.
fn qualify(entry: &ModelEntry, taken: &HashSet<String>) -> String {
    let own = entry.display_name.trim();
    let qualified = |qualifier: &str| format!("{own}{SEPARATOR}{qualifier}");
    if let Some(qualifier) = entry.metadata.as_ref().and_then(label_qualifier) {
        let name = qualified(qualifier);
        if !taken.contains(&name) {
            return name;
        }
    }
    let name = qualified(&entry.model_id);
    if !taken.contains(&name) {
        return name;
    }
    let mut count = 2;
    loop {
        let candidate = format!("{name} {count}");
        if !taken.contains(&candidate) {
            return candidate;
        }
        count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// One listed row: its id, the name the listing states, and the label the
    /// gateway states for it when it states one.
    fn entry(id: &str, name: &str, label: Option<&str>) -> ModelEntry {
        let metadata = label.map(|label| json!({"raw": {"magpie_label": label}}));
        serde_json::from_value(json!({
            "modelId": id,
            "displayName": name,
            "status": "active",
            "capabilities": {"modalities": ["text"], "toolCalling": true,
                "streaming": true, "structuredOutput": true, "vision": false,
                "audio": false, "reasoning": false},
            "metadata": metadata,
        }))
        .unwrap()
    }

    /// The four rows a gateway returns for one model it serves over three
    /// channels: the group it builds over them, then one row per channel, all
    /// named alike and each labelled with where it comes from.
    fn gateway() -> Vec<ModelEntry> {
        vec![
            entry("auto/glm-5.3", "GLM-5.3", Some("GLM-5.3 · routing group")),
            entry(
                "siliconflow/glm-5.3",
                "GLM-5.3",
                Some("GLM-5.3 · SiliconFlow"),
            ),
            entry(
                "volcengine/glm-5.3",
                "GLM-5.3",
                Some("GLM-5.3 · Volcengine Ark"),
            ),
            entry("zhipu/glm-5.3", "GLM-5.3", Some("GLM-5.3 · Zhipu")),
        ]
    }

    #[test]
    fn a_gateways_rows_take_their_labels_apart() {
        let names = unique(&gateway());
        assert_eq!(
            names,
            [
                "GLM-5.3 · routing group",
                "GLM-5.3 · SiliconFlow",
                "GLM-5.3 · Volcengine Ark",
                "GLM-5.3 · Zhipu",
            ],
            "the group row is told apart from the channels it spreads over"
        );
        assert_eq!(
            names.iter().collect::<HashSet<_>>().len(),
            4,
            "four rows, four names"
        );
    }

    #[test]
    fn a_provider_without_duplicates_keeps_every_name() {
        let entries = vec![
            entry("flash", "Flash", Some("Flash · Dim")),
            entry("plain", "Plain", None),
            entry("spaced", " Spaced ", Some("Spaced · Dim")),
        ];
        assert_eq!(
            unique(&entries),
            ["Flash", "Plain", " Spaced "],
            "no name is rewritten, the one with space around it included"
        );
    }

    #[test]
    fn names_that_agree_only_after_trimming_are_duplicates() {
        let entries = vec![
            entry("a", "GLM-5.3", Some("GLM-5.3 · SiliconFlow")),
            entry("b", " GLM-5.3 ", Some("GLM-5.3 · Zhipu")),
        ];
        assert_eq!(
            unique(&entries),
            ["GLM-5.3 · SiliconFlow", "GLM-5.3 · Zhipu"],
            "the two state one name, so both are qualified"
        );
    }

    #[test]
    fn a_row_nothing_can_choose_still_counts_as_a_duplicate() {
        let mut entries = gateway();
        let stated = unique(&entries);
        // Rows no picker offers: one its status excludes, one a stored disable
        // set does. The rule reads neither.
        entries[2].status = crate::model_catalog::ModelStatus::Unavailable;
        assert_eq!(
            unique(&entries),
            stated,
            "a row's status changes no name, its own included"
        );
    }

    #[test]
    fn rows_whose_labels_collide_take_their_ids() {
        let entries = vec![
            entry(
                "siliconflow/glm-5.3",
                "GLM-5.3",
                Some("GLM-5.3 · SiliconFlow"),
            ),
            entry("proxy/glm-5.3", "GLM-5.3", Some("GLM-5.3 · SiliconFlow")),
        ];
        assert_eq!(
            unique(&entries),
            ["GLM-5.3 · SiliconFlow", "GLM-5.3 · proxy/glm-5.3"],
            "the second row's id is what tells it apart"
        );
    }

    #[test]
    fn a_row_without_a_label_takes_its_id() {
        let entries = vec![
            entry("flash", "Flash", Some("Flash · Dim")),
            entry("other", "Flash", None),
        ];
        assert_eq!(
            unique(&entries),
            ["Flash · Dim", "Flash · other"],
            "a provider that states no origin still names each row apart"
        );
    }

    #[test]
    fn a_renamed_row_keeps_its_own_name() {
        let entries = vec![
            entry(
                "siliconflow/x",
                "V4.1 Flash (SiliconFlow)",
                Some("V4.1 Flash (SiliconFlow) · SiliconFlow"),
            ),
            entry(
                "volcengine/x",
                "V4.1 Flash (SiliconFlow)",
                Some("V4.1 Flash (Volcengine) · Volcengine"),
            ),
        ];
        assert_eq!(
            unique(&entries),
            [
                "V4.1 Flash (SiliconFlow) · SiliconFlow",
                "V4.1 Flash (SiliconFlow) · Volcengine",
            ],
            "the name the user chose stays in front; only the origin is added"
        );
    }

    #[test]
    fn a_name_another_row_states_survives_as_it_stands() {
        let entries = vec![
            entry("a", "GLM-5.3", None),
            entry("b", "GLM-5.3", None),
            entry("c", "GLM-5.3 · b", None),
        ];
        let names = unique(&entries);
        assert_eq!(
            names,
            ["GLM-5.3 · a", "GLM-5.3 · b 2", "GLM-5.3 · b"],
            "the row that states one of the qualified names keeps it"
        );
        assert_eq!(
            names.iter().collect::<HashSet<_>>().len(),
            3,
            "the qualified names stay apart"
        );
    }

    #[test]
    fn a_row_whose_id_is_taken_takes_a_counter() {
        let entries = vec![
            entry("b", "GLM-5.3", None),
            entry("b", "GLM-5.3", None),
            entry("b", "GLM-5.3", None),
        ];
        assert_eq!(
            unique(&entries),
            ["GLM-5.3 · b", "GLM-5.3 · b 2", "GLM-5.3 · b 3"],
            "ids name one row each, so only a repeated id needs the counter"
        );
    }

    #[test]
    fn a_label_is_read_as_the_text_after_its_last_separator() {
        let labelled: ModelMetadata = serde_json::from_value(json!({
            "raw": {"magpie_label": "GLM-5.3 · SiliconFlow · Extra"}
        }))
        .unwrap();
        assert_eq!(label_qualifier(&labelled), Some("Extra"));
        let bare: ModelMetadata =
            serde_json::from_value(json!({"raw": {"magpie_label": "GLM-5.3"}})).unwrap();
        assert_eq!(label_qualifier(&bare), None, "a bare name states no origin");
        assert_eq!(
            label_qualifier(&ModelMetadata::default()),
            None,
            "a listing with no label states no origin"
        );
    }
}
