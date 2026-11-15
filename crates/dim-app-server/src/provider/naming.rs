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

/// The name a row publishes when it keeps the origin its listing states for it:
/// its own name, trimmed, followed by that origin.
fn label_candidate(entry: &ModelEntry) -> Option<String> {
    let qualifier = entry.metadata.as_ref().and_then(label_qualifier)?;
    Some(format!(
        "{}{SEPARATOR}{qualifier}",
        entry.display_name.trim()
    ))
}

/// The name each of one provider's complete rows publishes, in their order,
/// each unique within that provider. Every row takes part — a disabled or an
/// unavailable one too — so switching a row on or off renames no other, and
/// every client reads the same names.
///
/// A name compares after trimming. A row whose name no other row states keeps
/// it as stated. A row that states a shared name publishes its label candidate
/// — `<its own name> · <the origin its gateway states for it>` — but only while
/// no other row reaches the same candidate and no row states that name itself;
/// every other row of the group appends its id instead, so a label two rows
/// carry apart names neither of them. A name that is still taken gains a
/// counter: that is the one step a listing order can decide, and only rows
/// sharing an id reach it. Everything else is read off the whole set before any
/// name is assigned, so listing the same rows in another order names each row
/// the same.
pub(crate) fn unique<'a>(entries: impl IntoIterator<Item = &'a ModelEntry>) -> Vec<String> {
    let entries: Vec<&ModelEntry> = entries.into_iter().collect();
    let mut stated: HashMap<&str, usize> = HashMap::new();
    for entry in &entries {
        *stated.entry(entry.display_name.trim()).or_default() += 1;
    }

    // A shared row's candidate, and how many rows reach it. Both are read from
    // the whole set first, so what one row publishes cannot depend on the order
    // the rows arrive in.
    let candidates: Vec<Option<String>> = entries
        .iter()
        .map(|entry| {
            if stated[entry.display_name.trim()] > 1 {
                label_candidate(entry)
            } else {
                None
            }
        })
        .collect();
    let mut reached: HashMap<&str, usize> = HashMap::new();
    for candidate in candidates.iter().flatten() {
        *reached.entry(candidate.as_str()).or_default() += 1;
    }

    // Every name a row states is claimed: a computed name may not equal one
    // after trimming either, or two rows would publish one name.
    let mut claimed: HashSet<String> = stated.keys().map(|name| (*name).to_owned()).collect();
    let keeps_label: Vec<bool> = candidates
        .iter()
        .map(|candidate| {
            candidate.as_ref().is_some_and(|candidate| {
                reached[candidate.as_str()] == 1 && !claimed.contains(candidate.as_str())
            })
        })
        .collect();
    // A candidate a shared row keeps is claimed before any other row is named,
    // so an id a row appends cannot take it first.
    for (index, candidate) in candidates.iter().enumerate() {
        if keeps_label[index] {
            claimed.insert(candidate.clone().expect("a kept candidate is present"));
        }
    }

    let mut names = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let own = entry.display_name.trim();
        if stated[own] == 1 {
            names.push(entry.display_name.clone());
            continue;
        }
        let name = match keeps_label[index] {
            true => candidates[index]
                .clone()
                .expect("a kept candidate is present"),
            false => {
                let base = format!("{own}{SEPARATOR}{}", entry.model_id);
                let name = free(&base, &claimed);
                claimed.insert(name.clone());
                name
            }
        };
        names.push(name);
    }
    names
}

/// `base`, or the first `<base> <n>` no row publishes yet.
fn free(base: &str, claimed: &HashSet<String>) -> String {
    if !claimed.contains(base) {
        return base.to_owned();
    }
    let mut count = 2;
    loop {
        let candidate = format!("{base} {count}");
        if !claimed.contains(&candidate) {
            return candidate;
        }
        count += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

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

    /// The name each row publishes, by id, so an answer can be compared across
    /// the listing orders that hold the same rows.
    fn by_id(entries: &[ModelEntry]) -> BTreeMap<String, String> {
        unique(entries)
            .into_iter()
            .zip(entries)
            .map(|(name, entry)| (entry.model_id.clone(), name))
            .collect()
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
    fn rows_whose_labels_collide_all_take_their_ids() {
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
            ["GLM-5.3 · siliconflow/glm-5.3", "GLM-5.3 · proxy/glm-5.3",],
            "both rows carry the label, so it names neither and both take ids"
        );
    }

    #[test]
    fn the_names_do_not_depend_on_the_order_of_the_rows() {
        for mut entries in [
            vec![
                entry(
                    "siliconflow/glm-5.3",
                    "GLM-5.3",
                    Some("GLM-5.3 · SiliconFlow"),
                ),
                entry("proxy/glm-5.3", "GLM-5.3", Some("GLM-5.3 · SiliconFlow")),
            ],
            vec![
                entry("a", "GLM-5.3", None),
                entry("b", "GLM-5.3", None),
                entry("c", "GLM-5.3 · b", None),
            ],
            vec![
                entry("flash", "Flash", Some("Flash · Dim")),
                entry("other", "Flash", None),
            ],
            gateway(),
        ] {
            let forward = by_id(&entries);
            entries.reverse();
            assert_eq!(
                forward,
                by_id(&entries),
                "the same rows in another order name each row the same"
            );
        }
    }

    #[test]
    fn a_stated_name_that_differs_only_by_trimming_still_collides() {
        let entries = vec![
            entry("a", "GLM-5.3", None),
            entry("b", "GLM-5.3", None),
            entry("c", "GLM-5.3 · b ", None),
        ];
        assert_eq!(
            unique(&entries),
            ["GLM-5.3 · a", "GLM-5.3 · b 2", "GLM-5.3 · b "],
            "the row stating the name keeps it, space and all, so the id the \
             other row would take is already read as taken"
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
