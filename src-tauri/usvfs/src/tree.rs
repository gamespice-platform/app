//! The virtual tree: what the game sees when it looks at its own folder.
//!
//! This is the whole brain of the virtual-filesystem strategy, and it is
//! deliberately platform-independent — a lookup table from a path the game asks
//! for to a real path on disk, plus the directory listings that make the
//! redirected files *discoverable*.
//!
//! WHY LISTINGS ARE HALF THE WORK
//! ------------------------------
//! Redirecting `CreateFile` alone is not enough and it is the mistake every
//! first attempt makes. A mod loader does not open `mods/cool.jar` by name — it
//! *enumerates* `mods/` and opens whatever it finds. So the tree has to answer
//! two questions:
//!
//!   * **"open this exact path"** — resolve one virtual path to one real file;
//!   * **"what is in this directory"** — merge the real directory's own entries
//!     with every virtual entry underneath it, with the virtual ones winning.
//!
//! A tree that only did the first would make a modded game look completely
//! unmodded while every individual file was, technically, redirectable.
//!
//! CASE
//! ----
//! Windows paths are case-insensitive, and a game that writes `Mods\Cool.Jar`
//! must find a file staged as `mods/cool.jar`. Every key is therefore lowercased
//! and `\` is normalised to `/`. The stored value keeps its original case,
//! because that is what gets opened and Windows is the only platform this runs
//! on where the difference does not matter.
//!
//! LISTINGS
//! --------
//! [`VirtualTree::children_absolute`] says what a directory gains,
//! [`wildcard_match`] applies a `FindFirstFile` pattern the way Windows does,
//! and [`merge_listing`] folds the two together with the real directory's own
//! entries — all pure, so the hook half only has to turn the answer into
//! `WIN32_FIND_DATAW` records.
//!
//! WHAT IS NOT HERE
//! ----------------
//! Writes. A game that creates a file inside a virtualised directory writes it
//! to the REAL directory, unredirected — see the module header on
//! [`crate::hooks`] for why that is the right call rather than a gap.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// Cap on entries in one tree. A large Skyrim list is tens of thousands of
/// loose files; this has room and is still bounded.
pub const MAX_ENTRIES: usize = 250_000;

/// Cap on one path. Windows' own limit is 32,767 with the `\\?\` prefix.
pub const MAX_PATH_LEN: usize = 4096;

/// One virtual file: a path the game may ask for, and the real file behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mapping {
    /// Relative to the game directory, `/`-separated, original case.
    pub virtual_path: String,
    /// Absolute, on disk, original case.
    pub real_path: String,
}

/// The merged view of one game directory.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VirtualTree {
    /// The game directory itself, absolute. Every virtual path is relative to
    /// this and a path outside it is not virtualised at all.
    pub root: String,

    /// Lowercased virtual path → the real file.
    ///
    /// A `BTreeMap` rather than a hash map: the serialised form has to be
    /// deterministic so that two identical trees produce identical bytes, which
    /// is what lets the injected process notice a change by comparing a
    /// revision rather than diffing.
    files: BTreeMap<String, Mapping>,

    /// Lowercased directory path → the child NAMES it gains, original case.
    ///
    /// Derived from `files` and stored rather than recomputed, because the
    /// injected process answers `FindFirstFile` from it on a hot path and
    /// walking every mapping per call would be quadratic in the worst case a
    /// mod loader routinely produces.
    dirs: BTreeMap<String, BTreeSet<String>>,
}

impl VirtualTree {
    pub fn new(root: impl Into<String>) -> Self {
        Self {
            root: normalise_separators(&root.into()),
            files: BTreeMap::new(),
            dirs: BTreeMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Add one file. A later insert for the same path wins, which is how
    /// priority is expressed — the caller adds in ascending priority order.
    pub fn insert(&mut self, virtual_path: &str, real_path: &str) -> bool {
        let Some(clean) = clean_relative(virtual_path) else {
            return false;
        };

        if real_path.len() > MAX_PATH_LEN || real_path.is_empty() {
            return false;
        }

        let key = clean.to_ascii_lowercase();

        /*
         * The cap counts NEW paths only. Refusing a replacement at the cap
         * would drop an override rather than an addition — the caller inserts
         * in ascending priority order, so the entry being refused is the
         * higher-priority one, and the tree would quietly resolve to the file
         * the user's load order says loses.
         */
        if !self.files.contains_key(&key) && self.files.len() >= MAX_ENTRIES {
            return false;
        }

        // Every ancestor directory gains this entry's next component, so a
        // listing of `mods` finds `cool.jar` and a listing of the root finds
        // `mods`.
        let parts: Vec<&str> = clean.split('/').collect();

        for depth in 0..parts.len() {
            let dir = parts[..depth].join("/");
            let child = parts[depth].to_string();

            self.dirs
                .entry(dir.to_ascii_lowercase())
                .or_default()
                .insert(child);
        }

        self.files.insert(
            key,
            Mapping {
                virtual_path: clean,
                real_path: normalise_separators(real_path),
            },
        );

        true
    }

    /// The real file behind a virtual path, or `None`.
    ///
    /// Takes an ABSOLUTE path as the game would pass it, and answers `None` for
    /// anything outside the root — a game opens hundreds of files that have
    /// nothing to do with its own directory, and every one of them has to fall
    /// through untouched and fast.
    pub fn resolve_absolute(&self, path: &str) -> Option<&str> {
        let relative = self.relative_of(path)?;

        self.files
            .get(&relative.to_ascii_lowercase())
            .map(|m| m.real_path.as_str())
    }

    /// The real file behind a path already known to be relative to the root.
    pub fn resolve(&self, relative: &str) -> Option<&str> {
        let clean = clean_relative(relative)?;

        self.files
            .get(&clean.to_ascii_lowercase())
            .map(|m| m.real_path.as_str())
    }

    /// The names a directory gains, for a merged listing.
    ///
    /// Empty for a directory the tree adds nothing to, which is the answer for
    /// almost every directory a game enumerates.
    pub fn extra_entries(&self, relative_dir: &str) -> &BTreeSet<String> {
        static EMPTY: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();

        let key = clean_relative_dir(relative_dir).to_ascii_lowercase();

        self.dirs
            .get(&key)
            .unwrap_or_else(|| EMPTY.get_or_init(BTreeSet::new))
    }

    /// The same, from an absolute path.
    pub fn extra_entries_absolute(&self, dir: &str) -> &BTreeSet<String> {
        static EMPTY: std::sync::OnceLock<BTreeSet<String>> = std::sync::OnceLock::new();

        match self.relative_of(dir) {
            Some(relative) => self.extra_entries(&relative),
            None => EMPTY.get_or_init(BTreeSet::new),
        }
    }

    /// Does the tree add anything at or under this absolute directory?
    ///
    /// The cheap guard a hook uses before doing anything expensive.
    pub fn touches(&self, dir: &str) -> bool {
        self.relative_of(dir).is_some_and(|rel| {
            self.dirs
                .contains_key(&clean_relative_dir(&rel).to_ascii_lowercase())
        })
    }

    /// Is this absolute path a directory the tree CREATES — one that exists
    /// only because a mod ships something under it?
    ///
    /// True for the root and for every ancestor of a mapping. Whether the real
    /// directory also exists is the caller's question; this says only that the
    /// tree vouches for it, which is what lets `GetFileAttributesW` answer
    /// "directory" for a `mods` folder nobody has made yet.
    pub fn is_virtual_dir_absolute(&self, path: &str) -> bool {
        self.touches(path)
    }

    /// What an absolute directory gains, with what each name is.
    ///
    /// A name that has children of its own is a directory; one that is a
    /// mapping is a file carrying its real path. A name that is both (a file
    /// staged as `a` and another staged as `a/b`) is a directory, since the
    /// deeper entry cannot exist otherwise.
    pub fn children_absolute(&self, dir: &str) -> Vec<VirtualChild> {
        let Some(relative) = self.relative_of(dir) else {
            return Vec::new();
        };

        let base = clean_relative_dir(&relative);

        self.extra_entries(&base)
            .iter()
            .map(|name| {
                let child = if base.is_empty() {
                    name.clone()
                } else {
                    format!("{base}/{name}")
                };

                let key = child.to_ascii_lowercase();

                let kind = if self.dirs.contains_key(&key) {
                    ChildKind::Dir
                } else if let Some(mapping) = self.files.get(&key) {
                    ChildKind::File {
                        real_path: mapping.real_path.clone(),
                    }
                } else {
                    ChildKind::Dir
                };

                VirtualChild {
                    name: name.clone(),
                    kind,
                }
            })
            .collect()
    }

    /// Every mapping, for tests and for the report.
    pub fn mappings(&self) -> impl Iterator<Item = &Mapping> {
        self.files.values()
    }

    /// An absolute path expressed relative to the root, or `None` when it is
    /// outside.
    fn relative_of(&self, path: &str) -> Option<String> {
        let path = normalise_separators(path);

        // `\\?\C:\Games\X` and `C:\Games\X` are the same file, and a game
        // handed the first by its own launcher would otherwise miss every
        // redirect.
        let path = path
            .strip_prefix("//?/UNC/")
            .map(|rest| format!("//{rest}"))
            .unwrap_or_else(|| {
                path.strip_prefix("//?/")
                    .map(str::to_string)
                    .unwrap_or(path)
            });

        let lower = path.to_ascii_lowercase();
        let root = self.root.to_ascii_lowercase();

        let rest = lower.strip_prefix(&root)?;

        // The root itself, or a path under it — but not a sibling whose name
        // merely starts with the root's (`C:/Games/X2` under `C:/Games/X`).
        let rest = match rest.chars().next() {
            None => "",
            Some('/') => &rest[1..],
            Some(_) => return None,
        };

        // Take the ORIGINAL case back: the value is what gets opened.
        let start = path.len() - rest.len();

        Some(path[start..].to_string())
    }
}

/// One name a virtual directory gains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualChild {
    /// Original case, as staged.
    pub name: String,
    pub kind: ChildKind,
}

/// What a [`VirtualChild`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildKind {
    /// A mapped file, with the real path to read it from.
    File { real_path: String },
    /// A directory the tree adds entries under.
    Dir,
}

/// One entry of the real directory, as far as merging cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RealEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Where one line of a merged listing comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// The real directory's entry at this index of the slice passed in.
    Real(usize),
    /// A virtual file; read its metadata from this real path.
    VirtualFile(String),
    /// A virtual directory with no real counterpart.
    VirtualDir,
}

/// One line of a merged listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedEntry {
    pub name: String,
    pub source: Source,
}

/// Split a path at its last separator: `("C:\\g\\mods", "*.jar")`.
///
/// Both separators count, because a game mixes them. `None` when there is no
/// separator or nothing after it — `FindFirstFile` on `dir\` is an error, not
/// a listing, so the hook must not invent one.
pub fn split_last_component(path: &str) -> Option<(&str, &str)> {
    let at = path.rfind(['\\', '/'])?;
    let (dir, rest) = path.split_at(at);
    let name = &rest[1..];

    if name.is_empty() {
        return None;
    }

    Some((dir, name))
}

/// Can [`wildcard_match`] answer this pattern faithfully?
///
/// The DOS wildcards (`<`, `>`, `"`) are an NTFS-only extension of the
/// `FindFirstFile` syntax that nothing in the JDK or a mod loader emits. A
/// pattern using them is passed to the real API unmerged rather than
/// approximated.
pub fn pattern_supported(pattern: &str) -> bool {
    !pattern.contains(['<', '>', '"'])
}

/// `FindFirstFile`'s pattern matching: `*` any run, `?` any one character,
/// case-insensitive, and `*.*` meaning everything (Windows treats it as `*`,
/// so `README` matches it despite having no dot).
pub fn wildcard_match(pattern: &str, name: &str) -> bool {
    let pattern = if pattern == "*.*" { "*" } else { pattern };

    let pattern: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let name: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();

    let (mut p, mut n) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;

    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if let Some((star_p, star_n)) = star {
            // Backtrack: let the last `*` swallow one more character.
            p = star_p + 1;
            n = star_n + 1;
            star = Some((star_p, star_n + 1));
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }

    p == pattern.len()
}

/// Fold a directory's virtual entries into its real ones.
///
/// Rules, which are the whole behaviour:
///   * only names matching `pattern` appear (real `.` and `..` included, as
///     the real API returns them);
///   * names compare case-insensitively;
///   * a virtual FILE replaces a real entry of the same name — it is what an
///     open of that path resolves to, so the listing must agree with it;
///   * a virtual DIRECTORY never replaces a real directory (the real entry
///     keeps its own metadata, and its contents merge when that directory is
///     listed in turn), but it does replace a real file of that name;
///   * real entries keep their order, virtual-only entries follow in name
///     order, and nothing appears twice.
pub fn merge_listing(
    real: &[RealEntry],
    virtual_children: &[VirtualChild],
    pattern: &str,
) -> Vec<MergedEntry> {
    let mut out: Vec<MergedEntry> = Vec::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();

    for (index, entry) in real.iter().enumerate() {
        if !wildcard_match(pattern, &entry.name) {
            continue;
        }

        // A real directory listing never repeats a name, but a buggy filter
        // driver might; first wins.
        if seen.contains_key(&entry.name.to_lowercase()) {
            continue;
        }

        seen.insert(entry.name.to_lowercase(), out.len());

        out.push(MergedEntry {
            name: entry.name.clone(),
            source: Source::Real(index),
        });
    }

    for child in virtual_children {
        if !wildcard_match(pattern, &child.name) {
            continue;
        }

        let key = child.name.to_lowercase();

        let merged = MergedEntry {
            name: child.name.clone(),
            source: match &child.kind {
                ChildKind::File { real_path } => Source::VirtualFile(real_path.clone()),
                ChildKind::Dir => Source::VirtualDir,
            },
        };

        match seen.get(&key).copied() {
            Some(position) => {
                let real_is_dir = match &out[position].source {
                    Source::Real(index) => real[*index].is_dir,
                    _ => false,
                };

                let keep_real = matches!(child.kind, ChildKind::Dir) && real_is_dir;

                if !keep_real {
                    out[position] = merged;
                }
            }
            None => {
                seen.insert(key, out.len());
                out.push(merged);
            }
        }
    }

    out
}

/// `\` → `/`, and collapse repeats.
///
/// Repeats matter: `C:\Games\\X` is a path Windows accepts and a naive prefix
/// comparison rejects.
fn normalise_separators(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    let mut last_sep = false;

    for ch in path.chars() {
        let sep = ch == '\\' || ch == '/';

        if sep {
            // A leading `//` is a UNC prefix and is kept; everything else
            // collapses.
            if last_sep && !out.is_empty() && out.len() > 1 {
                continue;
            }

            out.push('/');
            last_sep = true;
        } else {
            out.push(ch);
            last_sep = false;
        }
    }

    // A trailing separator makes `mods/` and `mods` different keys.
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }

    out
}

/// A relative virtual path, or `None` if it is not one.
///
/// Refuses `..`, absolutes and drive prefixes for the same reason the plugin
/// jail does: this string is joined onto a directory and handed to the
/// operating system.
fn clean_relative(path: &str) -> Option<String> {
    if path.is_empty() || path.len() > MAX_PATH_LEN || path.contains('\0') {
        return None;
    }

    let normalised = normalise_separators(path);

    // An absolute path, a drive letter, or a UNC root.
    if normalised.starts_with('/') || normalised.chars().nth(1) == Some(':') {
        return None;
    }

    let mut parts: Vec<&str> = Vec::new();

    for part in normalised.split('/') {
        match part {
            "" | "." => continue,
            ".." => return None,
            other => parts.push(other),
        }
    }

    if parts.is_empty() {
        return None;
    }

    Some(parts.join("/"))
}

/// The same, for a directory — where the empty string is the root and legal.
fn clean_relative_dir(path: &str) -> String {
    clean_relative(path).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> VirtualTree {
        let mut tree = VirtualTree::new("C:/Games/Skyrim");

        tree.insert(
            "Data/textures/sky.dds",
            "D:/staging/a/Data/textures/sky.dds",
        );
        tree.insert("Data/mod.esp", "D:/staging/a/Data/mod.esp");

        tree
    }

    #[test]
    fn a_mapped_path_resolves_to_its_real_file() {
        let tree = tree();

        assert_eq!(
            tree.resolve("Data/mod.esp"),
            Some("D:/staging/a/Data/mod.esp")
        );

        assert_eq!(
            tree.resolve_absolute("C:/Games/Skyrim/Data/mod.esp"),
            Some("D:/staging/a/Data/mod.esp")
        );
    }

    /// Windows paths are case-insensitive, and a game that writes
    /// `Data\Mod.ESP` must find a file staged as `Data/mod.esp`.
    #[test]
    fn lookups_ignore_case_and_separator() {
        let tree = tree();

        for asked in [
            "C:/Games/Skyrim/Data/mod.esp",
            "C:\\Games\\Skyrim\\Data\\Mod.ESP",
            "c:/games/skyrim/data/MOD.esp",
            "C:\\Games\\Skyrim\\\\Data\\mod.esp",
        ] {
            assert_eq!(
                tree.resolve_absolute(asked),
                Some("D:/staging/a/Data/mod.esp"),
                "{asked}"
            );
        }
    }

    /// A launcher that hands the game an extended-length path would otherwise
    /// miss every redirect.
    #[test]
    fn an_extended_length_prefix_is_understood() {
        let tree = tree();

        assert_eq!(
            tree.resolve_absolute("\\\\?\\C:\\Games\\Skyrim\\Data\\mod.esp"),
            Some("D:/staging/a/Data/mod.esp")
        );
    }

    /// A game opens hundreds of files that have nothing to do with its own
    /// folder. Every one has to fall through untouched.
    #[test]
    fn a_path_outside_the_root_is_not_virtualised() {
        let tree = tree();

        for outside in [
            "C:/Windows/System32/kernel32.dll",
            "D:/staging/a/Data/mod.esp",
            // The dangerous one: a sibling whose name starts with the root's.
            "C:/Games/Skyrim2/Data/mod.esp",
            "C:/Games/Skyrim.bak/Data/mod.esp",
        ] {
            assert_eq!(tree.resolve_absolute(outside), None, "{outside}");
        }
    }

    /// Redirecting `CreateFile` alone makes a modded game look unmodded: a mod
    /// loader ENUMERATES its directory rather than opening files by name.
    #[test]
    fn every_ancestor_directory_gains_its_child() {
        let tree = tree();

        assert!(tree.extra_entries("").contains("Data"));
        assert!(tree.extra_entries("Data").contains("mod.esp"));
        assert!(tree.extra_entries("Data").contains("textures"));
        assert!(tree.extra_entries("Data/textures").contains("sky.dds"));

        // And by the case the game happens to use.
        assert!(tree.extra_entries("DATA").contains("mod.esp"));

        // A directory the tree adds nothing to.
        assert!(tree.extra_entries("Data/meshes").is_empty());
    }

    #[test]
    fn listings_work_from_an_absolute_directory_too() {
        let tree = tree();

        assert!(tree
            .extra_entries_absolute("C:\\Games\\Skyrim\\Data")
            .contains("mod.esp"));

        assert!(tree
            .extra_entries_absolute("C:/Windows/System32")
            .is_empty());
    }

    #[test]
    fn touches_is_the_cheap_guard_a_hook_needs() {
        let tree = tree();

        assert!(tree.touches("C:/Games/Skyrim"));
        assert!(tree.touches("C:/Games/Skyrim/Data"));
        assert!(!tree.touches("C:/Games/Skyrim/Data/meshes"));
        assert!(!tree.touches("C:/Windows"));
    }

    /// The caller adds in ascending priority order, so a later insert wins —
    /// which is how the merge tree's own priority reaches this one.
    #[test]
    fn a_later_insert_wins_the_same_path() {
        let mut tree = VirtualTree::new("C:/Games/X");

        tree.insert("Data/sky.dds", "D:/low/sky.dds");
        tree.insert("Data/sky.dds", "D:/high/sky.dds");

        assert_eq!(tree.resolve("Data/sky.dds"), Some("D:/high/sky.dds"));
        assert_eq!(tree.len(), 1);
    }

    /// The string is joined onto a directory and handed to the OS, so it gets
    /// the same refusals the plugin jail applies.
    #[test]
    fn an_unsafe_virtual_path_is_refused() {
        let mut tree = VirtualTree::new("C:/Games/X");

        for bad in [
            "../escape.dll",
            "Data/../../escape.dll",
            "/absolute",
            "C:/absolute",
            "\\\\server\\share\\x",
            "",
            "..",
        ] {
            assert!(!tree.insert(bad, "D:/staging/x"), "{bad} should be refused");
        }

        assert!(tree.is_empty());
    }

    #[test]
    fn a_tree_is_bounded() {
        let mut tree = VirtualTree::new("C:/Games/X");

        assert!(tree.insert("a.txt", "D:/x/a.txt"));

        let long = "x".repeat(MAX_PATH_LEN + 1);

        assert!(
            !tree.insert("b.txt", &long),
            "an absurd real path is refused"
        );
        assert!(
            !tree.insert(&long, "D:/x/b.txt"),
            "an absurd virtual path is refused"
        );

        assert_eq!(tree.len(), 1, "neither refusal left a partial entry");
    }

    /// The entry cap, exercised for real. Slow enough to be worth saying why it
    /// is here: the blob is mapped into somebody else's game, so "the tree grew
    /// until the process died" has to be impossible rather than unlikely.
    #[test]
    fn the_entry_cap_is_enforced() {
        let mut tree = VirtualTree::new("C:/Games/X");

        for n in 0..MAX_ENTRIES {
            assert!(tree.insert(&format!("d/f{n}"), "D:/x/f"), "entry {n}");
        }

        assert_eq!(tree.len(), MAX_ENTRIES);
        assert!(!tree.insert("d/one-too-many", "D:/x/f"));

        // A path already present is a REPLACEMENT, not a new entry, so it is
        // still accepted at the cap — otherwise a full tree could never be
        // corrected.
        assert!(tree.insert("d/f0", "D:/y/f"));
        assert_eq!(tree.resolve("d/f0"), Some("D:/y/f"));
    }

    #[test]
    fn separators_normalise_without_eating_a_unc_prefix() {
        assert_eq!(normalise_separators("C:\\Games\\X"), "C:/Games/X");
        assert_eq!(normalise_separators("C:\\Games\\\\X\\"), "C:/Games/X");
        assert_eq!(normalise_separators("//server/share"), "//server/share");
        assert_eq!(normalise_separators("mods/"), "mods");
    }

    /// The serialised form has to be deterministic, so the injected process can
    /// notice a change by comparing a revision rather than diffing a tree.
    #[test]
    fn two_identical_trees_serialise_identically() {
        let mut a = VirtualTree::new("C:/Games/X");
        let mut b = VirtualTree::new("C:/Games/X");

        // Inserted in different orders.
        a.insert("z.txt", "D:/x/z.txt");
        a.insert("a.txt", "D:/x/a.txt");

        b.insert("a.txt", "D:/x/a.txt");
        b.insert("z.txt", "D:/x/z.txt");

        assert_eq!(
            serde_json::to_string(&a).expect("a"),
            serde_json::to_string(&b).expect("b")
        );
    }

    // ---------------------------------------------------------- listings

    fn mods_tree() -> VirtualTree {
        let mut tree = VirtualTree::new("C:/Games/MC");

        tree.insert("mods/Cool.jar", "D:/staging/a/mods/Cool.jar");
        tree.insert("mods/sub/inner.jar", "D:/staging/a/mods/sub/inner.jar");
        tree.insert("config/x.toml", "D:/staging/a/config/x.toml");

        tree
    }

    fn names(merged: &[MergedEntry]) -> Vec<&str> {
        merged.iter().map(|m| m.name.as_str()).collect()
    }

    fn file(name: &str) -> RealEntry {
        RealEntry {
            name: name.into(),
            is_dir: false,
        }
    }

    fn dir(name: &str) -> RealEntry {
        RealEntry {
            name: name.into(),
            is_dir: true,
        }
    }

    #[test]
    fn children_say_what_is_a_file_and_what_is_a_directory() {
        let tree = mods_tree();
        let children = tree.children_absolute("C:\\Games\\MC\\mods");

        assert_eq!(children.len(), 2);

        assert!(children.iter().any(|c| c.name == "Cool.jar"
            && c.kind
                == ChildKind::File {
                    real_path: "D:/staging/a/mods/Cool.jar".into()
                }));
        assert!(children
            .iter()
            .any(|c| c.name == "sub" && c.kind == ChildKind::Dir));

        // The root gains the two top-level directories.
        let root = tree.children_absolute("c:/games/mc");
        assert_eq!(root.len(), 2);
        assert!(root.iter().all(|c| c.kind == ChildKind::Dir));

        // Outside the root, and a directory the tree adds nothing to.
        assert!(tree.children_absolute("C:/Windows").is_empty());
        assert!(tree.children_absolute("C:/Games/MC/saves").is_empty());
    }

    #[test]
    fn a_directory_exists_virtually_when_a_mod_ships_something_under_it() {
        let tree = mods_tree();

        assert!(tree.is_virtual_dir_absolute("C:\\Games\\MC\\MODS\\sub"));
        assert!(tree.is_virtual_dir_absolute("C:/Games/MC"));

        // A file is not a directory; neither is something unmapped.
        assert!(!tree.is_virtual_dir_absolute("C:/Games/MC/mods/Cool.jar"));
        assert!(!tree.is_virtual_dir_absolute("C:/Games/MC/saves"));
    }

    #[test]
    fn a_path_splits_at_its_last_separator_of_either_kind() {
        assert_eq!(
            split_last_component("C:\\g\\mods\\*.jar"),
            Some(("C:\\g\\mods", "*.jar"))
        );
        assert_eq!(
            split_last_component("C:/g\\mods/a.jar"),
            Some(("C:/g\\mods", "a.jar"))
        );

        // No separator, or nothing after it: not a search.
        assert_eq!(split_last_component("mods"), None);
        assert_eq!(split_last_component("C:\\g\\mods\\"), None);
    }

    #[test]
    fn wildcards_follow_findfirstfile() {
        assert!(wildcard_match("*", "anything.jar"));
        assert!(wildcard_match("*.jar", "Cool.JAR"));
        assert!(!wildcard_match("*.jar", "cool.zip"));
        assert!(wildcard_match("c?ol.jar", "COOL.jar"));
        assert!(!wildcard_match("c?ol.jar", "col.jar"));
        assert!(wildcard_match("*o*l*", "cool"));
        assert!(!wildcard_match("*o*l*x", "cool"));
        assert!(wildcard_match("exact.txt", "Exact.TXT"));
        assert!(!wildcard_match("exact.txt", "exact.txt2"));

        // `*.*` means everything, dot or not.
        assert!(wildcard_match("*.*", "README"));
        assert!(wildcard_match("*", ".."));

        // Backtracking over a repeated prefix.
        assert!(wildcard_match("*ab", "aab"));
        assert!(wildcard_match("a*a*b", "aaab"));

        assert!(pattern_supported("*.jar"));
        assert!(!pattern_supported("<.jar"));
    }

    #[test]
    fn virtual_entries_join_the_real_ones() {
        let tree = mods_tree();
        let children = tree.children_absolute("C:/Games/MC/mods");

        let real = [dir("."), dir(".."), file("local.jar")];

        let merged = merge_listing(&real, &children, "*");

        assert_eq!(names(&merged), [".", "..", "local.jar", "Cool.jar", "sub"]);
        assert_eq!(merged[2].source, Source::Real(2));
        assert_eq!(
            merged[3].source,
            Source::VirtualFile("D:/staging/a/mods/Cool.jar".into())
        );
        assert_eq!(merged[4].source, Source::VirtualDir);
    }

    #[test]
    fn a_virtual_file_replaces_a_real_one_of_the_same_name_ignoring_case() {
        let tree = mods_tree();
        let children = tree.children_absolute("C:/Games/MC/mods");

        let real = [file("cool.JAR")];
        let merged = merge_listing(&real, &children, "*");

        // One entry, not two, and it is the virtual one.
        assert_eq!(
            merged
                .iter()
                .filter(|m| m.name.eq_ignore_ascii_case("cool.jar"))
                .count(),
            1
        );
        assert!(matches!(merged[0].source, Source::VirtualFile(_)));
    }

    #[test]
    fn a_virtual_directory_never_hides_a_real_one() {
        let tree = mods_tree();
        let children = tree.children_absolute("C:/Games/MC/mods");

        let real = [dir("SUB")];
        let merged = merge_listing(&real, &children, "sub");

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, Source::Real(0));

        // But it does replace a real FILE of that name.
        let merged = merge_listing(&[file("sub")], &children, "sub");
        assert_eq!(merged[0].source, Source::VirtualDir);
    }

    #[test]
    fn the_pattern_filters_both_halves() {
        let tree = mods_tree();
        let children = tree.children_absolute("C:/Games/MC/mods");

        let real = [file("a.jar"), file("b.txt")];

        let merged = merge_listing(&real, &children, "*.jar");
        assert_eq!(names(&merged), ["a.jar", "Cool.jar"]);

        // An exact name is a one-line listing: how `FindFirstFile` doubles as
        // a stat.
        let merged = merge_listing(&real, &children, "cool.jar");
        assert_eq!(names(&merged), ["Cool.jar"]);

        let merged = merge_listing(&real, &children, "missing.jar");
        assert!(merged.is_empty());
    }

    #[test]
    fn a_directory_that_exists_only_virtually_lists_without_any_real_entries() {
        let tree = mods_tree();
        let children = tree.children_absolute("C:/Games/MC/config");

        let merged = merge_listing(&[], &children, "*");

        assert_eq!(names(&merged), ["x.toml"]);
    }

    #[test]
    fn a_subdirectory_lists_its_own_level_only() {
        let tree = mods_tree();
        let children = tree.children_absolute("C:/Games/MC/mods/sub");

        let merged = merge_listing(&[], &children, "*");

        assert_eq!(names(&merged), ["inner.jar"]);
    }
}
