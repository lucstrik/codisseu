//! Ordered collection operations. Destinations are collection/folder IDs in this workspace.
use crate::*;
fn find<'a>(items: &'a [CollectionItem], id: &str) -> Option<&'a CollectionItem> {
    items.iter().find_map(|i| {
        if i.id() == id {
            Some(i)
        } else if let CollectionItem::Folder { items, .. } = i {
            find(items, id)
        } else {
            None
        }
    })
}
fn find_mut<'a>(items: &'a mut [CollectionItem], id: &str) -> Option<&'a mut CollectionItem> {
    items.iter_mut().find_map(|i| {
        if i.id() == id {
            Some(i)
        } else if let CollectionItem::Folder { items, .. } = i {
            find_mut(items, id)
        } else {
            None
        }
    })
}
fn contains(item: &CollectionItem, id: &str) -> bool {
    item.id() == id || matches!(item,CollectionItem::Folder{items,..} if find(items,id).is_some())
}
fn take(items: &mut Vec<CollectionItem>, id: &str) -> Option<CollectionItem> {
    if let Some(index) = items.iter().position(|i| i.id() == id) {
        return Some(items.remove(index));
    }
    items.iter_mut().find_map(|i| {
        if let CollectionItem::Folder { items, .. } = i {
            take(items, id)
        } else {
            None
        }
    })
}
fn siblings<'a>(
    items: &'a mut Vec<CollectionItem>,
    id: &str,
) -> Option<&'a mut Vec<CollectionItem>> {
    if items.iter().any(|i| i.id() == id) {
        return Some(items);
    }
    items.iter_mut().find_map(|i| {
        if let CollectionItem::Folder { items, .. } = i {
            siblings(items, id)
        } else {
            None
        }
    })
}
fn depth(item: &CollectionItem) -> usize {
    match item {
        CollectionItem::Request(_) => 0,
        CollectionItem::Folder { items, .. } => 1 + items.iter().map(depth).max().unwrap_or(0),
    }
}
fn count(items: &[CollectionItem]) -> usize {
    items
        .iter()
        .map(|i| {
            1 + if let CollectionItem::Folder { items, .. } = i {
                count(items)
            } else {
                0
            }
        })
        .sum()
}
fn reidentify(item: &mut CollectionItem) {
    match item {
        CollectionItem::Request(r) => r.id = new_id(),
        CollectionItem::Folder { id, items, .. } => {
            *id = new_id();
            for i in items {
                reidentify(i);
            }
        }
    }
}
impl ApiWorkspace {
    pub fn parent_id(&self, id: &str) -> Option<String> {
        fn walk(items: &[CollectionItem], id: &str, parent: &str) -> Option<String> {
            for i in items {
                if i.id() == id {
                    return Some(parent.into());
                }
                if let CollectionItem::Folder {
                    id: folder, items, ..
                } = i
                    && let Some(found) = walk(items, id, folder)
                {
                    return Some(found);
                }
            }
            None
        }
        self.collections
            .iter()
            .find_map(|c| walk(&c.items, id, &c.id))
    }
    pub(crate) fn item_count(&self) -> usize {
        self.collections.iter().map(|c| count(&c.items)).sum()
    }
    pub fn item(&self, id: &str) -> Option<&CollectionItem> {
        self.collections.iter().find_map(|c| find(&c.items, id))
    }
    pub fn item_mut(&mut self, id: &str) -> Option<&mut CollectionItem> {
        self.collections
            .iter_mut()
            .find_map(|c| find_mut(&mut c.items, id))
    }
    pub fn destination_mut(&mut self, id: &str) -> Option<&mut Vec<CollectionItem>> {
        for c in &mut self.collections {
            if c.id == id {
                return Some(&mut c.items);
            }
            if let Some(CollectionItem::Folder { items, .. }) = find_mut(&mut c.items, id) {
                return Some(items);
            }
        }
        None
    }
    /// Full labels distinguish identically named folders in different services.
    pub fn destinations(&self, exclude: Option<&str>) -> Vec<(String, String, usize)> {
        fn walk(
            items: &[CollectionItem],
            base: &str,
            level: usize,
            exclude: Option<&str>,
            out: &mut Vec<(String, String, usize)>,
        ) {
            for i in items {
                if Some(i.id()) == exclude {
                    continue;
                }
                if let CollectionItem::Folder {
                    id, name, items, ..
                } = i
                {
                    let label = format!("{base} / {name}");
                    out.push((id.clone(), label.clone(), level));
                    walk(items, &label, level + 1, exclude, out);
                }
            }
        }
        let mut out = vec![];
        for c in &self.collections {
            out.push((c.id.clone(), c.name.clone(), 0));
            walk(&c.items, &c.name, 1, exclude, &mut out);
        }
        out
    }
    pub fn new_folder(&mut self, parent: &str, name: impl Into<String>) -> Result<String, String> {
        if self.item_count() >= 10_000 {
            return Err("Workspace exceeds 10000 items".into());
        }
        let level = self
            .destinations(None)
            .into_iter()
            .find(|(id, _, _)| id == parent)
            .ok_or("Destination no longer exists")?
            .2;
        if level >= 31 {
            return Err("Folder depth exceeds 32".into());
        }
        let id = new_id();
        self.destination_mut(parent)
            .ok_or("Destination no longer exists")?
            .push(CollectionItem::Folder {
                id: id.clone(),
                name: name.into(),
                items: vec![],
                auth: Auth::Inherit,
                pre_request_script: String::new(),
                post_response_script: String::new(),
            });
        Ok(id)
    }
    pub fn move_item(&mut self, id: &str, parent: &str) -> Result<(), String> {
        let item = self.item(id).ok_or("Item no longer exists")?;
        if contains(item, parent) {
            return Err("A folder cannot be moved into itself or its descendants".into());
        }
        let level = self
            .destinations(None)
            .into_iter()
            .find(|(id, _, _)| id == parent)
            .ok_or("Destination no longer exists")?
            .2;
        if level + depth(item) > 32 {
            return Err("Folder depth exceeds 32".into());
        }
        let item = self
            .collections
            .iter_mut()
            .find_map(|c| take(&mut c.items, id))
            .ok_or("Item no longer exists")?;
        self.destination_mut(parent)
            .expect("validated destination")
            .push(item);
        Ok(())
    }
    pub fn duplicate_item(&mut self, id: &str) -> Result<String, String> {
        let mut duplicate = self.item(id).ok_or("Item no longer exists")?.clone();
        if self.item_count()
            + 1
            + if let CollectionItem::Folder { items, .. } = &duplicate {
                count(items)
            } else {
                0
            }
            > 10_000
        {
            return Err("Workspace exceeds 10000 items".into());
        }
        reidentify(&mut duplicate);
        match &mut duplicate {
            CollectionItem::Request(r) => r.name.push_str(" copy"),
            CollectionItem::Folder { name, .. } => name.push_str(" copy"),
        }
        let duplicate_id = duplicate.id().to_owned();
        let target = self
            .collections
            .iter_mut()
            .find_map(|c| siblings(&mut c.items, id))
            .ok_or("Item no longer exists")?;
        let index = target
            .iter()
            .position(|i| i.id() == id)
            .expect("validated sibling");
        target.insert(index + 1, duplicate);
        Ok(duplicate_id)
    }
    pub fn reorder_item(&mut self, id: &str, down: bool) -> bool {
        let Some(target) = self
            .collections
            .iter_mut()
            .find_map(|c| siblings(&mut c.items, id))
        else {
            return false;
        };
        let Some(index) = target.iter().position(|i| i.id() == id) else {
            return false;
        };
        let other = if down {
            index + 1
        } else {
            index.wrapping_sub(1)
        };
        if other >= target.len() {
            return false;
        }
        target.swap(index, other);
        true
    }
    pub fn request_path(&self, id: &str) -> Vec<String> {
        fn walk(items: &[CollectionItem], id: &str, path: &mut Vec<String>) -> bool {
            for i in items {
                path.push(i.name().to_owned());
                if i.id() == id {
                    return true;
                }
                if let CollectionItem::Folder { items, .. } = i
                    && walk(items, id, path)
                {
                    return true;
                }
                path.pop();
            }
            false
        }
        for c in &self.collections {
            let mut path = vec![c.name.clone()];
            if walk(&c.items, id, &mut path) {
                return path;
            }
        }
        vec![]
    }
}
