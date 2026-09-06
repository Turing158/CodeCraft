use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

/// Keeps the newest ordinary inbox entries while never deleting entries the
/// caller identifies as pending user interaction. When the priority set alone
/// exceeds the cap, it is retained so an approval/question cannot be lost.
pub(crate) fn limit_paths<F>(paths: Vec<PathBuf>, maximum: usize, is_priority: F) -> Vec<PathBuf>
where
    F: Fn(&Path) -> bool,
{
    let mut entries = paths
        .into_iter()
        .map(|path| {
            let modified = fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH);
            (path, modified, false)
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        left.1
            .cmp(&right.1)
            .then_with(|| left.0.cmp(&right.0))
    });

    if entries.len() > maximum {
        for (path, _, priority) in &mut entries {
            *priority = is_priority(path);
        }
        let remove_count = entries.len() - maximum;
        let mut removable = entries
            .iter()
            .enumerate()
            .filter(|(_, (_, _, priority))| !priority)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        for index in removable.drain(..remove_count.min(removable.len())) {
            let _ = fs::remove_file(&entries[index].0);
            entries[index].0 = PathBuf::new();
        }
        entries.retain(|(path, _, _)| !path.as_os_str().is_empty());
    }

    entries
        .into_iter()
        .map(|(path, _, _)| path)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::limit_paths;

    #[test]
    fn removes_oldest_ordinary_entries_but_keeps_priority_entries() {
        let root = std::env::temp_dir().join(format!(
            "codecraft-inbox-limit-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let paths = ["01-old.json", "02-priority.json", "03-new.json"]
            .into_iter()
            .map(|name| {
                let path = root.join(name);
                fs::write(&path, b"{}").unwrap();
                path
            })
            .collect();

        let kept = limit_paths(paths, 2, |path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains("priority"))
        });

        assert_eq!(kept.len(), 2);
        assert!(kept.iter().any(|path| path.ends_with("02-priority.json")));
        assert!(!root.join("01-old.json").exists());
        assert!(root.join("02-priority.json").exists());
        assert!(root.join("03-new.json").exists());
        for path in kept {
            let _ = fs::remove_file(path);
        }
        let _ = fs::remove_dir(root);
    }
}
