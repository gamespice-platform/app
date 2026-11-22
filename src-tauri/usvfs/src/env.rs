//! Environment blocks, for the child-process hook.
//!
//! `CreateProcessW` takes the child's environment as one flat block of
//! `NAME=value` strings, each NUL-terminated, the whole thing ended by an empty
//! string. A game that passes its own block (the Minecraft launcher and most
//! JVM wrappers do) would hand the child an environment without the two
//! variables the hook DLL reads at attach, and the child would start
//! unhooked. This module rebuilds such a block with those variables present.
//!
//! Pure and platform-independent so it is tested on every host; the Windows
//! hook only supplies the raw block.

/// One `NAME=value` entry, without its terminator.
pub type Entry = Vec<u16>;

/// Split a wide block into its entries.
///
/// Stops at the first empty entry, as Windows does. `block` may be longer than
/// the real block (a caller passing a slice of a bigger buffer); anything after
/// the terminator is ignored. A block missing its terminator yields the
/// entries up to the end of the slice.
pub fn parse_wide(block: &[u16]) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut start = 0usize;

    for (index, unit) in block.iter().enumerate() {
        if *unit != 0 {
            continue;
        }

        if index == start {
            return out;
        }

        out.push(block[start..index].to_vec());
        start = index + 1;
    }

    if start < block.len() {
        out.push(block[start..].to_vec());
    }

    out
}

/// The name part of an entry. A name may itself begin with `=` (the per-drive
/// `=C:=C:\dir` entries), so the separator is the first `=` after position 0.
fn name_of(entry: &[u16]) -> &[u16] {
    let from = usize::from(entry.first() == Some(&(b'=' as u16)));

    match entry[from..].iter().position(|unit| *unit == b'=' as u16) {
        Some(at) => &entry[..from + at],
        None => entry,
    }
}

fn fold(name: &[u16]) -> String {
    String::from_utf16_lossy(name).to_uppercase()
}

/// Serialise entries into a wide block, sorted by name the way Windows
/// documents, with the final terminating empty entry.
fn to_block(mut entries: Vec<Entry>) -> Vec<u16> {
    entries.sort_by_key(|entry| fold(name_of(entry)));

    let mut block: Vec<u16> = Vec::new();

    for entry in entries {
        block.extend(entry);
        block.push(0);
    }

    // An empty block is two NULs, not one.
    if block.is_empty() {
        block.push(0);
    }

    block.push(0);

    block
}

/// `entries` with each of `additions` set: an existing entry of the same name
/// (compared case-insensitively, as Windows does) is replaced rather than
/// duplicated.
pub fn with_additions(entries: Vec<Entry>, additions: &[(&str, &str)]) -> Vec<u16> {
    let wanted: Vec<String> = additions
        .iter()
        .map(|(name, _)| name.to_uppercase())
        .collect();

    let mut kept: Vec<Entry> = entries
        .into_iter()
        .filter(|entry| !wanted.contains(&fold(name_of(entry))))
        .collect();

    for (name, value) in additions {
        kept.push(format!("{name}={value}").encode_utf16().collect());
    }

    to_block(kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().collect()
    }

    fn block(entries: &[&str]) -> Vec<u16> {
        let mut out = Vec::new();

        for entry in entries {
            out.extend(wide(entry));
            out.push(0);
        }

        out.push(0);

        out
    }

    fn texts(block: &[u16]) -> Vec<String> {
        parse_wide(block)
            .iter()
            .map(|entry| String::from_utf16_lossy(entry))
            .collect()
    }

    #[test]
    fn a_block_splits_into_its_entries_and_stops_at_the_terminator() {
        let mut raw = block(&["A=1", "B=2"]);

        // Junk after the terminator is another buffer's business.
        raw.extend(wide("C=3"));

        assert_eq!(texts(&raw), vec!["A=1", "B=2"]);
    }

    #[test]
    fn a_block_without_a_terminator_still_yields_what_is_there() {
        let raw = wide("A=1");

        assert_eq!(texts(&raw), vec!["A=1"]);
    }

    #[test]
    fn an_empty_block_has_no_entries() {
        assert!(parse_wide(&[0, 0]).is_empty());
        assert!(parse_wide(&[]).is_empty());
    }

    #[test]
    fn additions_are_appended_in_sorted_position() {
        let merged = with_additions(
            parse_wide(&block(&["PATH=x", "ZED=1"])),
            &[("TMC_VFS", "v")],
        );

        assert_eq!(texts(&merged), vec!["PATH=x", "TMC_VFS=v", "ZED=1"]);
        assert!(merged.ends_with(&[0, 0]));
    }

    #[test]
    fn an_existing_entry_is_replaced_case_insensitively() {
        let merged = with_additions(
            parse_wide(&block(&["tmc_vfs=old", "PATH=x"])),
            &[("TMC_VFS", "new")],
        );

        assert_eq!(texts(&merged), vec!["PATH=x", "TMC_VFS=new"]);
    }

    #[test]
    fn a_drive_entry_beginning_with_equals_keeps_its_name_and_sorts_first() {
        let merged = with_additions(
            parse_wide(&block(&["=C:=C:\\dir", "A=1"])),
            &[("TMC_VFS", "v")],
        );

        assert_eq!(texts(&merged)[0], "=C:=C:\\dir");
        assert_eq!(texts(&merged).len(), 3);
    }

    #[test]
    fn an_empty_environment_becomes_just_the_additions() {
        let merged = with_additions(Vec::new(), &[("A", "1")]);

        assert_eq!(texts(&merged), vec!["A=1"]);
    }

    #[test]
    fn nothing_at_all_is_still_a_valid_double_nul_block() {
        assert_eq!(with_additions(Vec::new(), &[]), vec![0, 0]);
    }
}
