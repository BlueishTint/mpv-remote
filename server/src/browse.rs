//! Read-only file browsing for the phone, confined to configured roots.

use std::cmp::Ordering;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

const MEDIA_EXT: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "webm", "mov", "ts", "m2ts", "flv", "wmv", "mpg", "mpeg", "ogv",
    "mp3", "flac", "ogg", "opus", "m4a", "wav", "aac", "wma",
];

#[derive(Serialize)]
pub struct Entry {
    name: String,
    dir: bool,
    size: u64,
}

#[derive(Serialize)]
pub struct Listing {
    path: String,
    parent: Option<String>,
    entries: Vec<Entry>,
}

/// Canonical directories the phone may look into (and load files from).
pub struct Roots(Vec<PathBuf>);

impl Roots {
    /// `list` is `|`-separated; empty means the user's home directory. Folders that don't exist are
    /// skipped (main.lua warns about them), without widening access to the home directory.
    pub fn new(list: &str) -> Self {
        if list.trim().is_empty() {
            let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
            return Self(home.and_then(|h| fs::canonicalize(h).ok()).into_iter().collect());
        }
        Self(list.split('|').filter_map(|s| fs::canonicalize(s.trim()).ok()).collect())
    }

    pub fn first(&self) -> Option<&Path> {
        self.0.first().map(PathBuf::as_path)
    }

    /// Canonical form of `path` if it lies inside a root or inside `extra` (the playing file's folder).
    pub fn allows(&self, path: &Path, extra: Option<&Path>) -> Option<PathBuf> {
        let canon = fs::canonicalize(path).ok()?;
        let extra = extra.and_then(|e| fs::canonicalize(e).ok());
        let inside = self.0.iter().chain(extra.iter()).any(|r| canon.starts_with(r));
        inside.then_some(canon)
    }

    pub fn list(&self, path: &Path, extra: Option<&Path>) -> Option<Listing> {
        let dir = self.allows(path, extra).filter(|p| p.is_dir())?;
        let mut entries: Vec<Entry> = fs::read_dir(&dir)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    return None;
                }
                let meta = fs::metadata(e.path()).ok()?; // follows symlinks, unlike e.metadata()
                let is_dir = meta.is_dir();
                let is_media = Path::new(&name)
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| MEDIA_EXT.contains(&x.to_ascii_lowercase().as_str()));
                (is_dir || is_media).then_some(Entry { name, dir: is_dir, size: meta.len() })
            })
            .collect();
        entries.sort_by(|a, b| match (a.dir, b.dir) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });
        let parent = dir.parent().and_then(|p| self.allows(p, extra)).map(|p| display(&p));
        Some(Listing { path: display(&dir), parent, entries })
    }
}

/// Canonical Windows paths carry a `\\?\` (or `\\?\UNC\` for network shares) prefix that mpv and phones shouldn't see.
pub fn display(p: &Path) -> String {
    let s = p.to_string_lossy();
    if let Some(share) = s.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{share}");
    }
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_strips_verbatim_prefixes() {
        assert_eq!(display(Path::new(r"\\?\C:\Videos")), r"C:\Videos");
        assert_eq!(display(Path::new(r"\\?\UNC\nas\media\a.mkv")), r"\\nas\media\a.mkv");
        assert_eq!(display(Path::new("/home/me")), "/home/me");
    }

    #[test]
    fn allows_only_inside_roots() {
        let root = std::env::temp_dir().join("mpv-remote-test-root");
        fs::create_dir_all(root.join("sub")).unwrap();
        let roots = Roots::new(&root.to_string_lossy());
        assert!(roots.allows(&root.join("sub"), None).is_some());
        assert!(roots.allows(&root.join("sub").join("..").join(".."), None).is_none());
        assert!(roots.allows(&std::env::temp_dir(), None).is_none());
        assert!(roots.allows(&std::env::temp_dir(), Some(&std::env::temp_dir())).is_some());
    }

    #[test]
    fn missing_roots_do_not_fall_back_to_home() {
        assert!(Roots::new("Z:/does/not/exist|/nor/this").first().is_none());
    }
}
