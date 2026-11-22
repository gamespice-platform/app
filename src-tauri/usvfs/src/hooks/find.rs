//! Virtual directory listings: `FindFirstFileW`, `FindFirstFileExW`,
//! `FindNextFileW`, `FindClose`.
//!
//! The merge itself (which names appear, who wins a collision, how patterns
//! match) is [`crate::tree::merge_listing`], pure and tested on every
//! platform. This file is the Windows plumbing around it, and it is
//! type-checked only: it has never run.
//!
//! HOW A MERGED LISTING IS SERVED
//! ------------------------------
//! The first call enumerates the real directory COMPLETELY (the real handle is
//! opened, drained and closed inside the hook), merges the tree's entries into
//! that, and keeps the result in a heap `Listing`. The handle handed back to
//! the game is the address of that `Listing`; `FindNextFileW` and `FindClose`
//! recognise their own handles by a registry and pass everything else through
//! untouched.
//!
//! WHEN THIS STAYS OUT OF THE WAY
//! ------------------------------
//! A listing is merged only when the info level is standard or basic, the
//! search is a plain name match, the pattern's directory part is an absolute
//! path the tree adds entries to, the pattern has no DOS wildcards, and at
//! least one added name matches. Otherwise the real API answers unmodified.
//!
//! WHAT A VIRTUAL ENTRY LOOKS LIKE
//! -------------------------------
//! A virtual FILE reports its staged file's size, times and attributes under
//! the name it was staged with. A virtual DIRECTORY with no real counterpart
//! reports `FILE_ATTRIBUTE_DIRECTORY` and zeroed times. The short (8.3) name
//! is always empty for virtual entries.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use windows_sys::Win32::Foundation::{GetLastError, SetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    FindClose, FindFirstFileExW, FindFirstFileW, FindNextFileW, FILE_ATTRIBUTE_DIRECTORY,
    WIN32_FIND_DATAW,
};

use crate::tree::{
    merge_listing, pattern_supported, split_last_component, wildcard_match, RealEntry, Source,
    VirtualChild,
};

use super::{from_wide, to_wide};

const INFO_STANDARD: i32 = 0;
const INFO_BASIC: i32 = 1;
const SEARCH_NAME_MATCH: i32 = 0;

const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_PATH_NOT_FOUND: u32 = 3;
const ERROR_NO_MORE_FILES: u32 = 18;
const ERROR_INVALID_PARAMETER: u32 = 87;

pub(crate) type FindFirstFileWFn =
    unsafe extern "system" fn(*const u16, *mut WIN32_FIND_DATAW) -> HANDLE;
pub(crate) type FindFirstFileExWFn =
    unsafe extern "system" fn(*const u16, i32, *mut c_void, i32, *const c_void, u32) -> HANDLE;
pub(crate) type FindNextFileWFn = unsafe extern "system" fn(HANDLE, *mut WIN32_FIND_DATAW) -> i32;
pub(crate) type FindCloseFn = unsafe extern "system" fn(HANDLE) -> i32;

pub(crate) static REAL_FIND_FIRST_FILE_W: AtomicUsize = AtomicUsize::new(0);
pub(crate) static REAL_FIND_FIRST_FILE_EX_W: AtomicUsize = AtomicUsize::new(0);
pub(crate) static REAL_FIND_NEXT_FILE_W: AtomicUsize = AtomicUsize::new(0);
pub(crate) static REAL_FIND_CLOSE: AtomicUsize = AtomicUsize::new(0);

/// A merged listing, owned by the game through its handle.
struct Listing {
    entries: Vec<WIN32_FIND_DATAW>,
    next: usize,
}

// The registry of live virtual handles. Taken only while at least one virtual
// handle is open (LIVE_COUNT is checked first) and never held across a call
// into the real API.
static LIVE: Mutex<Vec<usize>> = Mutex::new(Vec::new());
static LIVE_COUNT: AtomicUsize = AtomicUsize::new(0);

fn registry() -> std::sync::MutexGuard<'static, Vec<usize>> {
    LIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn is_virtual(handle: HANDLE) -> bool {
    if LIVE_COUNT.load(Ordering::Acquire) == 0 {
        return false;
    }

    registry().contains(&(handle as usize))
}

unsafe fn real_first_w() -> FindFirstFileWFn {
    match REAL_FIND_FIRST_FILE_W.load(Ordering::Acquire) {
        0 => FindFirstFileW,
        saved => std::mem::transmute::<usize, FindFirstFileWFn>(saved),
    }
}

unsafe fn real_first_ex() -> FindFirstFileExWFn {
    match REAL_FIND_FIRST_FILE_EX_W.load(Ordering::Acquire) {
        0 => std::mem::transmute::<*const c_void, FindFirstFileExWFn>(
            FindFirstFileExW as *const c_void,
        ),
        saved => std::mem::transmute::<usize, FindFirstFileExWFn>(saved),
    }
}

unsafe fn real_next() -> FindNextFileWFn {
    match REAL_FIND_NEXT_FILE_W.load(Ordering::Acquire) {
        0 => FindNextFileW,
        saved => std::mem::transmute::<usize, FindNextFileWFn>(saved),
    }
}

unsafe fn real_close() -> FindCloseFn {
    match REAL_FIND_CLOSE.load(Ordering::Acquire) {
        0 => FindClose,
        saved => std::mem::transmute::<usize, FindCloseFn>(saved),
    }
}

/// Our `FindFirstFileW`.
///
/// # Safety
///
/// Called by the game with `FindFirstFileW`'s contract.
pub(crate) unsafe extern "system" fn hooked_find_first_file_w(
    name: *const u16,
    data: *mut WIN32_FIND_DATAW,
) -> HANDLE {
    if let Some(handle) = try_merge(
        name,
        INFO_STANDARD,
        data as *mut c_void,
        SEARCH_NAME_MATCH,
        std::ptr::null(),
        0,
    ) {
        return handle;
    }

    real_first_w()(name, data)
}

/// Our `FindFirstFileExW`.
///
/// # Safety
///
/// Called by the game with `FindFirstFileExW`'s contract.
pub(crate) unsafe extern "system" fn hooked_find_first_file_ex_w(
    name: *const u16,
    level: i32,
    data: *mut c_void,
    search: i32,
    filter: *const c_void,
    flags: u32,
) -> HANDLE {
    if let Some(handle) = try_merge(name, level, data, search, filter, flags) {
        return handle;
    }

    real_first_ex()(name, level, data, search, filter, flags)
}

/// Our `FindNextFileW`.
///
/// # Safety
///
/// Called by the game with `FindNextFileW`'s contract. A virtual handle is only
/// produced here, so reading it back as a `Listing` is sound while the
/// registry lists it.
pub(crate) unsafe extern "system" fn hooked_find_next_file_w(
    handle: HANDLE,
    data: *mut WIN32_FIND_DATAW,
) -> i32 {
    if !is_virtual(handle) {
        return real_next()(handle, data);
    }

    if data.is_null() {
        SetLastError(ERROR_INVALID_PARAMETER);

        return 0;
    }

    let listing = &mut *(handle as *mut Listing);

    match listing.entries.get(listing.next) {
        Some(entry) => {
            *data = *entry;
            listing.next += 1;

            1
        }
        None => {
            SetLastError(ERROR_NO_MORE_FILES);

            0
        }
    }
}

/// Our `FindClose`.
///
/// # Safety
///
/// Called by the game with `FindClose`'s contract.
pub(crate) unsafe extern "system" fn hooked_find_close(handle: HANDLE) -> i32 {
    if !is_virtual(handle) {
        return real_close()(handle);
    }

    {
        let mut live = registry();

        if let Some(at) = live.iter().position(|entry| *entry == handle as usize) {
            live.swap_remove(at);

            LIVE_COUNT.fetch_sub(1, Ordering::AcqRel);
        }
    }

    drop(Box::from_raw(handle as *mut Listing));

    1
}

/// Serve a merged listing if this call is one, or `None` to pass it through.
///
/// # Safety
///
/// `name` must be a NUL-terminated wide string or null; `data` the caller's
/// find-data buffer.
unsafe fn try_merge(
    name: *const u16,
    level: i32,
    data: *mut c_void,
    search: i32,
    filter: *const c_void,
    flags: u32,
) -> Option<HANDLE> {
    let tree = super::tree()?;

    if name.is_null()
        || data.is_null()
        || (level != INFO_STANDARD && level != INFO_BASIC)
        || search != SEARCH_NAME_MATCH
    {
        return None;
    }

    let pattern = from_wide(name)?;

    let (dir, leaf) = split_last_component(&pattern)?;

    if !pattern_supported(leaf) {
        return None;
    }

    let added: Vec<VirtualChild> = tree
        .children_absolute(dir)
        .into_iter()
        .filter(|child| wildcard_match(leaf, &child.name))
        .collect();

    if added.is_empty() {
        return None;
    }

    // The real directory, drained. A directory that does not exist lists as
    // nothing, which is what a directory only the tree creates is.
    let mut real_data: Vec<WIN32_FIND_DATAW> = Vec::new();

    let mut first: WIN32_FIND_DATAW = std::mem::zeroed();

    let handle = real_first_ex()(
        name,
        level,
        &mut first as *mut WIN32_FIND_DATAW as *mut c_void,
        search,
        filter,
        flags,
    );

    if handle == INVALID_HANDLE_VALUE {
        let error = GetLastError();

        if error != ERROR_FILE_NOT_FOUND && error != ERROR_PATH_NOT_FOUND {
            SetLastError(error);

            return Some(handle);
        }
    } else {
        real_data.push(first);

        loop {
            let mut next: WIN32_FIND_DATAW = std::mem::zeroed();

            if real_next()(handle, &mut next) == 0 {
                break;
            }

            real_data.push(next);
        }

        real_close()(handle);
    }

    let real_entries: Vec<RealEntry> = real_data
        .iter()
        .map(|entry| RealEntry {
            name: name_of(entry),
            is_dir: entry.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0,
        })
        .collect();

    // The pattern was already applied to both sides.
    let merged = merge_listing(&real_entries, &added, "*");

    let mut entries: Vec<WIN32_FIND_DATAW> = Vec::with_capacity(merged.len());

    for line in &merged {
        match &line.source {
            Source::Real(index) => entries.push(real_data[*index]),
            Source::VirtualFile(real_path) => {
                if let Some(entry) = staged_entry(&line.name, real_path, level, filter, flags) {
                    entries.push(entry);
                }
            }
            Source::VirtualDir => {
                let mut entry: WIN32_FIND_DATAW = std::mem::zeroed();

                entry.dwFileAttributes = FILE_ATTRIBUTE_DIRECTORY;

                set_name(&mut entry, &line.name);

                entries.push(entry);
            }
        }
    }

    if entries.is_empty() {
        SetLastError(ERROR_FILE_NOT_FOUND);

        return Some(INVALID_HANDLE_VALUE);
    }

    *(data as *mut WIN32_FIND_DATAW) = entries[0];

    let raw = Box::into_raw(Box::new(Listing { entries, next: 1 })) as usize;

    {
        registry().push(raw);

        LIVE_COUNT.fetch_add(1, Ordering::AcqRel);
    }

    Some(raw as HANDLE)
}

/// The listing line for a staged file: its real metadata under its virtual
/// name. `None` when the staged file cannot be read, so a dangling mapping is
/// omitted rather than listed as something that cannot be opened.
///
/// # Safety
///
/// Calls the real find functions.
unsafe fn staged_entry(
    name: &str,
    real_path: &str,
    level: i32,
    filter: *const c_void,
    flags: u32,
) -> Option<WIN32_FIND_DATAW> {
    let wide = to_wide(real_path);

    let mut entry: WIN32_FIND_DATAW = std::mem::zeroed();

    let handle = real_first_ex()(
        wide.as_ptr(),
        level,
        &mut entry as *mut WIN32_FIND_DATAW as *mut c_void,
        SEARCH_NAME_MATCH,
        filter,
        flags,
    );

    if handle == INVALID_HANDLE_VALUE {
        return None;
    }

    real_close()(handle);

    set_name(&mut entry, name);

    Some(entry)
}

fn name_of(entry: &WIN32_FIND_DATAW) -> String {
    let len = entry
        .cFileName
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(entry.cFileName.len());

    String::from_utf16_lossy(&entry.cFileName[..len])
}

/// Write a name into an entry (truncated to what the struct holds) and clear
/// the short name, which belonged to the staged file and not to this path.
fn set_name(entry: &mut WIN32_FIND_DATAW, name: &str) {
    entry.cFileName = [0; 260];
    entry.cAlternateFileName = [0; 14];

    for (slot, unit) in entry
        .cFileName
        .iter_mut()
        .zip(name.encode_utf16().take(259))
    {
        *slot = unit;
    }
}
