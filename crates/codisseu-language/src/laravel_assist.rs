//! Routes narrowly recognized Laravel contexts through the shared LSP worker.
use crate::laravel::{components, eloquent_static, requests, resources};
use std::{
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
};

pub(crate) fn merge_completion(
    items: &mut Vec<crate::CompletionItem>,
    extras: Vec<crate::CompletionItem>,
    truncated: &mut bool,
) {
    // Keep a usable semantic item on collisions. Proven context-specific extras
    // must still be reachable when the provider already filled its 128 slots.
    let mut additions = Vec::new();
    for item in extras {
        if let Some(existing) = items.iter_mut().find(|i| i.label == item.label) {
            if existing.unsupported_reason.is_some() {
                *existing = item;
            }
        } else if !additions
            .iter()
            .any(|i: &crate::CompletionItem| i.label == item.label)
        {
            additions.push(item);
        }
    }
    additions.append(items);
    *truncated |= additions.len() > 128;
    additions.truncate(128);
    *items = additions;
}

pub(crate) struct Assistance {
    pub result: resources::ResultSet,
    // Eloquent augments semantic PHP completion rather than hiding its members.
    pub supplemental: bool,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn query(
    root: &Path,
    path: &Path,
    source: &str,
    line: u32,
    character: u32,
    cancel: &AtomicBool,
    buffers: &[(PathBuf, &str)],
    completion: bool,
) -> Option<Assistance> {
    // Specific component/request expressions precede generic resource strings.
    if let Some(result) = components::query(
        root, path, source, line, character, cancel, buffers, completion,
    ) {
        return Some(Assistance {
            result,
            supplemental: false,
        });
    }
    if let Some(result) = requests::query(
        root, path, source, line, character, cancel, buffers, completion,
    ) {
        return Some(Assistance {
            result,
            supplemental: completion && requests::property_context(source, line, character),
        });
    }
    if let Some(result) = resources::query(
        root, path, source, line, character, cancel, buffers, completion,
    ) {
        return Some(Assistance {
            result,
            supplemental: false,
        });
    }
    eloquent_static::query(
        root, path, source, line, character, cancel, buffers, completion,
    )
    .map(|result| Assistance {
        result,
        supplemental: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(label: &str, value: &str) -> crate::CompletionItem {
        crate::completion::decode(
            Some(&serde_json::json!([{"label":label,"insertText":value}])),
            "probe.php".into(),
            1,
            (0, 0),
            "",
        )
        .unwrap()
        .items
        .remove(0)
    }
    #[test]
    fn supplementary_members_preserve_semantic_edits_and_remain_reachable_at_the_cap() {
        let mut items = (0..128)
            .map(|n| item(&format!("member{n}"), "semantic"))
            .collect::<Vec<_>>();
        let mut truncated = false;
        merge_completion(
            &mut items,
            vec![item("member0", "inferred"), item("scopeActive", "active")],
            &mut truncated,
        );
        assert!(truncated);
        assert_eq!(items.len(), 128);
        assert!(
            items
                .iter()
                .any(|i| i.label == "scopeActive" && i.new_text == "active")
        );
        assert!(
            items
                .iter()
                .any(|i| i.label == "member0" && i.new_text == "semantic")
        );
        let mut blocked = item("email", "");
        blocked.unsupported_reason = Some("unsupported edit".into());
        let mut items = vec![blocked];
        merge_completion(&mut items, vec![item("email", "email")], &mut false);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].new_text, "email");
        assert!(items[0].unsupported_reason.is_none());
    }
}
