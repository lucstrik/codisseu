//! File mentions use an independent bounded filesystem worker, never the lazy
//! Explorer entries. Reuse Search Everywhere's traversal/ranking and stale-reply
//! handling rather than adding another scanner or doing filesystem I/O in egui.
use codisseu_core::search_everywhere::{
    Category, FILES, FileFilter, Request, SearchEverywhereService,
};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

const MAX_MENTIONS: usize = 12;

#[derive(Default)]
pub(super) struct MentionSearch {
    worker: Option<SearchEverywhereService>,
    key: Option<(PathBuf, String)>,
    refresh: u64,
    pub(super) results: Vec<PathBuf>,
    pub(super) error: Option<String>,
}

impl MentionSearch {
    /// Repeated calls for the same root/query are cheap. The caller cancels when
    /// closing the picker so reopening discovers newly-created project files.
    pub(super) fn request(&mut self, root: Option<&Path>, query: &str, ctx: &egui::Context) {
        let Some(root) = root else {
            self.cancel();
            self.poll(ctx);
            return;
        };
        let key = (root.to_path_buf(), query.to_owned());
        if self.key.as_ref() != Some(&key) {
            self.results.clear();
            self.error = None;
            let worker = self.worker.get_or_insert_with(SearchEverywhereService::new);
            // Invalid input also cancels the previous request, preventing it from
            // reappearing while an invalid replacement query is visible.
            worker.cancel();
            let request = Request {
                encoding_profile: None,
                root: Some(root.to_path_buf()),
                external_roots: Vec::new(),
                buffers: Vec::new(),
                query: query.to_owned(),
                category: Category::Files,
                filter: FileFilter::All,
                include_non_project: false,
                only_open: false,
                refresh: self.refresh,
                preview: false,
                contributors: FILES,
            };
            self.error = worker.submit(request).err();
            self.key = Some(key);
        }
        self.poll(ctx);
    }

    pub(super) fn poll(&mut self, ctx: &egui::Context) {
        let Some(worker) = self.worker.as_mut() else {
            return;
        };
        worker.poll();
        if let Some(result) = worker.result.take() {
            self.results = self.key.as_ref().map_or_else(Vec::new, |(root, _)| {
                result
                    .hits
                    .into_iter()
                    .filter_map(|hit| {
                        (hit.path != *root && hit.path.starts_with(root)).then_some(hit.path)
                    })
                    .take(MAX_MENTIONS)
                    .collect()
            });
        }
        if let Some(error) = worker.error.take() {
            self.error = Some(error);
        }
        if worker.pending() {
            ctx.request_repaint_after(Duration::from_millis(16));
        }
    }

    pub(super) fn pending(&self) -> bool {
        self.key.is_some()
            && self.error.is_none()
            && self
                .worker
                .as_ref()
                .is_some_and(SearchEverywhereService::pending)
    }

    pub(super) fn cancel(&mut self) {
        if self.key.take().is_some() {
            self.refresh = self.refresh.wrapping_add(1);
            if let Some(worker) = self.worker.as_mut() {
                worker.cancel();
            }
        }
        self.results.clear();
        self.error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::Instant};

    fn finish(search: &mut MentionSearch, ctx: &egui::Context) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            search.poll(ctx);
            if !search.pending() {
                break;
            }
            assert!(Instant::now() < deadline, "mention worker did not finish");
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(search.error.is_none(), "{:?}", search.error);
    }

    #[test]
    fn finds_files_without_explorer_expansion_and_respects_default_ignores() {
        let root = tempfile::tempdir().unwrap();
        let deep = root.path().join("src/never-expanded/nested");
        fs::create_dir_all(&deep).unwrap();
        let expected = deep.join("invoice.php");
        fs::write(&expected, "<?php // no content needed by filename search").unwrap();
        for ignored in [
            ".git",
            "target",
            ".codisseu",
            "node_modules",
            "vendor",
            ".venv",
            "venv",
            "dist",
            "build",
        ] {
            fs::create_dir(root.path().join(ignored)).unwrap();
            fs::write(root.path().join(ignored).join("invoice.php"), "ignored").unwrap();
        }
        let ctx = egui::Context::default();
        let mut search = MentionSearch::default();
        search.request(Some(root.path()), "invoice", &ctx);
        finish(&mut search, &ctx);
        assert_eq!(search.results, vec![expected]);
    }

    #[test]
    fn bounds_results_and_invalidates_root_query_and_cancelled_work() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        for i in 0..40 {
            fs::write(first.path().join(format!("first-{i:02}.ts")), "").unwrap();
        }
        let expected = second.path().join("second.php");
        fs::write(&expected, "").unwrap();
        let ctx = egui::Context::default();
        let mut search = MentionSearch::default();
        search.request(Some(first.path()), "first", &ctx);
        finish(&mut search, &ctx);
        assert_eq!(search.results.len(), MAX_MENTIONS);
        search.request(Some(first.path()), "missing", &ctx);
        search.request(Some(second.path()), "second", &ctx);
        assert!(search.results.is_empty());
        finish(&mut search, &ctx);
        assert_eq!(search.results, vec![expected.clone()]);
        search.request(Some(first.path()), "first", &ctx);
        search.cancel();
        search.poll(&ctx);
        assert!(search.results.is_empty());
        assert!(!search.pending());
        // Reopening the same root/query refreshes the filesystem inventory.
        let newer = second.path().join("new.php");
        fs::write(&newer, "").unwrap();
        search.request(Some(second.path()), "", &ctx);
        finish(&mut search, &ctx);
        assert_eq!(search.results.len(), 2);
        assert!(search.results.contains(&expected));
        assert!(search.results.contains(&newer));
        search.request(None, "", &ctx);
        assert!(search.results.is_empty());
    }

    #[test]
    fn invalid_query_cannot_repopulate_results_from_previous_job() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("old.ts"), "").unwrap();
        let ctx = egui::Context::default();
        let mut search = MentionSearch::default();
        search.request(Some(root.path()), "old", &ctx);
        search.request(Some(root.path()), &"x".repeat(513), &ctx);
        let deadline = Instant::now() + Duration::from_secs(5);
        while search
            .worker
            .as_ref()
            .is_some_and(SearchEverywhereService::pending)
        {
            search.poll(&ctx);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(search.results.is_empty());
        assert!(search.error.is_some());
        search.request(Some(root.path()), "old", &ctx);
        finish(&mut search, &ctx);
        assert_eq!(search.results.len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_do_not_escape_project_or_enter_the_candidates() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        fs::write(external.path().join("outside.php"), "").unwrap();
        fs::write(root.path().join("inside.php"), "").unwrap();
        symlink(external.path(), root.path().join("linked-directory")).unwrap();
        symlink(
            external.path().join("outside.php"),
            root.path().join("linked-file.php"),
        )
        .unwrap();
        let ctx = egui::Context::default();
        let mut search = MentionSearch::default();
        search.request(Some(root.path()), "", &ctx);
        finish(&mut search, &ctx);
        assert_eq!(search.results, vec![root.path().join("inside.php")]);
        let alias = external.path().join("root-alias");
        symlink(root.path(), &alias).unwrap();
        search.request(Some(&alias), "", &ctx);
        finish(&mut search, &ctx);
        assert!(search.results.is_empty());
    }
}
