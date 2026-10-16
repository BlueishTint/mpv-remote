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
    /// `list` is `|`-separated; empty falls back to the user's home directory.
    pub fn new(list: &str) -> Self {
        let mut roots: Vec<PathBuf> = list
            .split('|')
            .filter(|s| !s.trim().is_empty())
            .filter_map(|s| fs::canonicalize(s.trim()).ok())
            .collect();
        if roots.is_empty() {
            let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
            roots.extend(home.and_then(|h| fs::canonicalize(h).ok()));
        }
        Self(roots)
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
                let meta = e.metadata().ok()?;
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

/// Canonical Windows paths carry a `\\?\` prefix that mpv and phones shouldn't see.
pub fn display(p: &Path) -> String {
    let s = p.to_string_lossy();
    s.strip_prefix(r"\\?\").unwrap_or(&s).to_owned()
}
