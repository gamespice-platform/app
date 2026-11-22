//! The half that runs inside the game.
//!
//! **Windows only, and honestly labelled.** Everything in this file has been
//! type-checked against the Windows target and has never been run against a
//! real game — see the crate header and `docs/gaps.md`. Nothing else in the app
//! depends on it working: the strategy is opt-in per sandbox, marked
//! EXPERIMENTAL, and a deploy that cannot carry it refuses cleanly.
//!
//! WHAT IS HOOKED
//! --------------
//!
//! | Calls | Why |
//! | --- | --- |
//! | `CreateFileW` | opens of a virtual file are answered from staging |
//! | `GetFileAttributesW`, `GetFileAttributesExW` | existence, size and "is it a directory" — the JDK's `File.exists()`, `Files.exists`, `Files.size` |
//! | `FindFirstFileW`, `FindFirstFileExW`, `FindNextFileW`, `FindClose` | directory listings merge the tree's entries with the real ones ([`find`]) |
//! | `LoadLibraryA/W`, `LoadLibraryExA/W` | a module loaded later (the JVM loads `java.dll`, `nio.dll` and `jvm.dll` after start) is patched when it arrives |
//! | `CreateProcessW`, `CreateProcessAsUserW` | a child process is created suspended, given this DLL and the tree, then resumed ([`children`]) |
//!
//! HOW THE REDIRECTION WORKS
//! -------------------------
//! **Import Address Table patching**, not inline hooking. When a PE module
//! calls `CreateFileW`, it does so through a pointer in its own import table
//! that the loader filled in; overwriting that pointer redirects every such
//! call in that module. The alternative — rewriting the first instructions of
//! the real function and building a trampoline — needs an instruction-length
//! decoder, has to be right about every prologue Microsoft ships, and breaks
//! differently on every Windows update.
//!
//! EVERY loaded module is patched, not only the executable: a Java process's
//! file access happens in `java.dll`, `nio.dll` and the UCRT, none of which is
//! the exe. An import counts when it comes from `kernel32`, `kernelbase`,
//! `advapi32` or an `api-ms-win-*` API set, because which of those a module
//! links against depends on the Windows build it was linked on. This DLL's own
//! module is skipped: its calls to the real functions must not come back here.
//!
//! What IAT patching costs, stated plainly because it decides which games this
//! can work for:
//!
//!   * **A call through `GetProcAddress` is not redirected.** The pointer never
//!     came from an import table.
//!   * **A delay-loaded import is not redirected.** Its table is a different
//!     structure and is not walked.
//!   * **A direct `ntdll` call is not redirected.** Some engines and most
//!     anti-tamper layers do this.
//!   * **A module loaded LATER has to be patched when it arrives**, which is
//!     why the `LoadLibrary` family is hooked — the hook re-walks every loaded
//!     module after the real call, which also catches the dependencies the
//!     loader pulled in on its own.
//!
//! WHY ONLY READS
//! --------------
//! A file the game CREATES inside a virtualised directory is written to the
//! real directory, unredirected. That is a decision rather than a gap: a game
//! writing a save, a crash dump or a config into its own folder expects to find
//! it there again with every other tool, and a manager that quietly diverted
//! those into a staging folder would be one nobody could ever find their saves
//! in. Redirection applies to opening something that already exists virtually.
//! (A consequence that is a gap: opening a STAGED file for write writes the
//! staging copy, not a private one.)
//!
//! WHAT THE HOT PATH COSTS
//! -----------------------
//! The hooks run inside `CreateFileW`, thousands of times a second, on whatever
//! thread the game happens to be on — including, during startup, a loader
//! thread holding the loader lock. They take no lock and call nothing
//! re-entrant: the tree is read once at attach into an immutable structure and
//! every lookup after that is a read-only map probe. A path outside the game
//! directory costs one wide-to-UTF-8 conversion and a prefix compare. The one
//! lock in the crate is the registry of virtual find handles, taken only while
//! at least one such handle is open.

#![cfg(windows)]

pub mod children;
pub mod find;

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{GetLastError, SetLastError, FARPROC, HANDLE, HMODULE};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileAttributesExW, GetFileAttributesW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_MODE, INVALID_FILE_ATTRIBUTES, WIN32_FILE_ATTRIBUTE_DATA,
};
/*
 * `IMAGE_NT_HEADERS64` lives in `Diagnostics::Debug` but is gated behind the
 * `Win32_System_SystemInformation` FEATURE — the feature name and the module
 * path do not match, which is worth the note because the compiler's suggestion
 * sends you to a module that does not export it.
 */
use windows_sys::Win32::System::Diagnostics::Debug::{
    IMAGE_DIRECTORY_ENTRY_IMPORT, IMAGE_NT_HEADERS64,
};
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, LoadLibraryA, LoadLibraryExA, LoadLibraryExW,
    LoadLibraryW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};
use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_PROTECTION_FLAGS, PAGE_READWRITE};
use windows_sys::Win32::System::ProcessStatus::K32EnumProcessModules;
use windows_sys::Win32::System::SystemServices::{IMAGE_DOS_HEADER, IMAGE_IMPORT_DESCRIPTOR};
use windows_sys::Win32::System::Threading::GetCurrentProcess;
use windows_sys::Win32::System::WindowsProgramming::IMAGE_THUNK_DATA64;

use crate::tree::VirtualTree;

/// The tree, read once at attach.
///
/// A `OnceLock` rather than a mutex: it is written exactly once, before the
/// hooks are installed, and read from every thread forever after. A lock here
/// would be a lock taken inside `CreateFileW` on the game's render thread.
static TREE: OnceLock<VirtualTree> = OnceLock::new();

/// This DLL's own module handle, so the patcher can skip it.
static OWN_MODULE: AtomicUsize = AtomicUsize::new(0);

/// This DLL's path on disk, which is what gets injected into children.
static OWN_PATH: OnceLock<PathBuf> = OnceLock::new();

/*
 * The real functions, saved before their import entries were overwritten.
 *
 * `AtomicUsize` rather than `static mut`. The moment an import entry is patched
 * another thread can be inside the hook, so the write and the read genuinely
 * race — `static mut` would be undefined behaviour that happens to work, and
 * the compiler says so. A relaxed atomic is the correct primitive and costs
 * nothing on x86: the pointer is published with `Release` before the patch that
 * makes it reachable, and read with `Acquire`.
 *
 * Zero means "not installed", which is why the hooks fall back to calling the
 * genuine import directly rather than treating it as impossible.
 */
static REAL_CREATE_FILE_W: AtomicUsize = AtomicUsize::new(0);
static REAL_GET_FILE_ATTRIBUTES_W: AtomicUsize = AtomicUsize::new(0);
static REAL_GET_FILE_ATTRIBUTES_EX_W: AtomicUsize = AtomicUsize::new(0);
static REAL_LOAD_LIBRARY_A: AtomicUsize = AtomicUsize::new(0);
static REAL_LOAD_LIBRARY_W: AtomicUsize = AtomicUsize::new(0);
static REAL_LOAD_LIBRARY_EX_A: AtomicUsize = AtomicUsize::new(0);
static REAL_LOAD_LIBRARY_EX_W: AtomicUsize = AtomicUsize::new(0);

/*
 * The REAL signature, taken from `windows-sys` rather than approximated. The
 * fourth argument is a `SECURITY_ATTRIBUTES` pointer; declaring it as
 * `*const c_void` compiles on its own and then refuses the genuine
 * `CreateFileW` as a fallback — which is exactly how the mismatch was found,
 * by type-checking this file against the Windows target.
 */
type CreateFileWFn = unsafe extern "system" fn(
    *const u16,
    u32,
    FILE_SHARE_MODE,
    *const SECURITY_ATTRIBUTES,
    u32,
    FILE_FLAGS_AND_ATTRIBUTES,
    HANDLE,
) -> HANDLE;

type GetFileAttributesWFn = unsafe extern "system" fn(*const u16) -> u32;

type GetFileAttributesExWFn =
    unsafe extern "system" fn(*const u16, i32, *mut std::ffi::c_void) -> i32;

type LoadLibraryAFn = unsafe extern "system" fn(*const u8) -> HMODULE;
type LoadLibraryWFn = unsafe extern "system" fn(*const u16) -> HMODULE;
type LoadLibraryExAFn = unsafe extern "system" fn(*const u8, HANDLE, u32) -> HMODULE;
type LoadLibraryExWFn = unsafe extern "system" fn(*const u16, HANDLE, u32) -> HMODULE;

/// The tree, for the hook submodules.
pub(crate) fn tree() -> Option<&'static VirtualTree> {
    TREE.get()
}

/// This DLL's path, for the child-process hook.
pub(crate) fn own_path() -> Option<&'static std::path::Path> {
    OWN_PATH.get().map(PathBuf::as_path)
}

/// Load the tree and install the hooks.
///
/// Called from `DllMain` on attach. Returns `false` for every failure —
/// including "there is no tree", which is the ordinary case for a process that
/// was injected by mistake — and a `false` leaves the game running completely
/// unmodified rather than half-hooked.
pub fn install() -> bool {
    let Ok(path) = std::env::var(crate::shm::ENV_BLOB) else {
        return false;
    };

    let Ok(published) = crate::shm::read(std::path::Path::new(&path)) else {
        return false;
    };

    /*
     * A tree for a DIFFERENT game directory means this DLL landed in a process
     * it was not meant for — a helper, an installer. Hooking that process would
     * redirect files for something nobody asked about. (A CHILD of the game
     * carries the same root, so it passes.)
     */
    if let Ok(root) = std::env::var(crate::shm::ENV_ROOT) {
        if !published.tree.root.eq_ignore_ascii_case(&root) {
            return false;
        }
    }

    if TREE.set(published.tree).is_err() {
        // Already installed. Not an error — `DllMain` can be reached twice if
        // something loads the DLL a second time.
        return true;
    }

    record_own_module();

    patch_all_modules() > 0
}

/// Remember which module is this DLL, and where it lives.
fn record_own_module() {
    let mut module: HMODULE = std::ptr::null_mut();

    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            install as *const () as *const u16,
            &mut module,
        )
    };

    if found == 0 || module.is_null() {
        return;
    }

    OWN_MODULE.store(module as usize, Ordering::Release);

    let mut buffer = [0u16; 1024];

    let len = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) };

    // Zero is failure; a full buffer means the path was truncated, and a
    // truncated path injected into a child would load nothing.
    if len == 0 || len as usize >= buffer.len() {
        return;
    }

    let path = PathBuf::from(OsString::from_wide(&buffer[..len as usize]));

    let _ = OWN_PATH.set(path);
}

/// Our `CreateFileW`.
///
/// # Safety
///
/// Called by the operating system's own callers with their arguments. Every
/// pointer here is one Windows already validated for the real function, and the
/// only thing this adds is a read of a NUL-terminated wide string — bounded
/// below — before handing everything to the original.
unsafe extern "system" fn hooked_create_file_w(
    file_name: *const u16,
    desired_access: u32,
    share_mode: FILE_SHARE_MODE,
    security: *const SECURITY_ATTRIBUTES,
    creation: u32,
    flags: FILE_FLAGS_AND_ATTRIBUTES,
    template: HANDLE,
) -> HANDLE {
    let saved = REAL_CREATE_FILE_W.load(Ordering::Acquire);

    /*
     * Zero cannot happen — the pointer is published before the patch that makes
     * this function reachable — but calling through a null pointer inside the
     * game's file open would be the worst possible way to find out otherwise,
     * so the genuine import is the fallback.
     */
    let real: CreateFileWFn = if saved == 0 {
        CreateFileW
    } else {
        std::mem::transmute::<usize, CreateFileWFn>(saved)
    };

    if let Some(redirected) = redirect(file_name) {
        let wide = to_wide(&redirected);

        return real(
            wide.as_ptr(),
            desired_access,
            share_mode,
            security,
            creation,
            flags,
            template,
        );
    }

    real(
        file_name,
        desired_access,
        share_mode,
        security,
        creation,
        flags,
        template,
    )
}

/// Our `GetFileAttributesW`.
///
/// Hooked because a loader that enumerates a directory then asks each entry
/// whether it is a file would otherwise be told the virtual ones do not exist,
/// and because `File.exists()` on a directory only a mod creates (`mods` in a
/// fresh instance) has to say yes.
///
/// # Safety
///
/// As [`hooked_create_file_w`].
unsafe extern "system" fn hooked_get_file_attributes_w(file_name: *const u16) -> u32 {
    let saved = REAL_GET_FILE_ATTRIBUTES_W.load(Ordering::Acquire);

    let real: GetFileAttributesWFn = if saved == 0 {
        GetFileAttributesW
    } else {
        std::mem::transmute::<usize, GetFileAttributesWFn>(saved)
    };

    if let Some(redirected) = redirect(file_name) {
        let wide = to_wide(&redirected);

        return real(wide.as_ptr());
    }

    let attributes = real(file_name);

    if attributes == INVALID_FILE_ATTRIBUTES && missing_but_virtual_dir(file_name) {
        SetLastError(0);

        return FILE_ATTRIBUTE_DIRECTORY;
    }

    attributes
}

/// Our `GetFileAttributesExW` — the call `java.nio.file.Files` makes for
/// `exists`, `isDirectory`, `size` and `getLastModifiedTime`.
///
/// Only `GetFileExInfoStandard` (level 0) is a `WIN32_FILE_ATTRIBUTE_DATA`; any
/// other level is passed through untouched.
///
/// # Safety
///
/// As [`hooked_create_file_w`]; `info` is the caller's buffer for the level it
/// asked about.
unsafe extern "system" fn hooked_get_file_attributes_ex_w(
    file_name: *const u16,
    level: i32,
    info: *mut std::ffi::c_void,
) -> i32 {
    let saved = REAL_GET_FILE_ATTRIBUTES_EX_W.load(Ordering::Acquire);

    let real: GetFileAttributesExWFn = if saved == 0 {
        GetFileAttributesExW
    } else {
        std::mem::transmute::<usize, GetFileAttributesExWFn>(saved)
    };

    if level != 0 {
        return real(file_name, level, info);
    }

    if let Some(redirected) = redirect(file_name) {
        let wide = to_wide(&redirected);

        return real(wide.as_ptr(), level, info);
    }

    let ok = real(file_name, level, info);

    if ok == 0 && !info.is_null() && missing_but_virtual_dir(file_name) {
        // Zeroed times and size: the directory exists only in the tree, so
        // there is no real metadata to report.
        let data = info as *mut WIN32_FILE_ATTRIBUTE_DATA;

        std::ptr::write_bytes(data, 0, 1);

        (*data).dwFileAttributes = FILE_ATTRIBUTE_DIRECTORY;

        SetLastError(0);

        return 1;
    }

    ok
}

/// The real call failed with "not found": is this nonetheless a directory the
/// tree creates?
///
/// # Safety
///
/// As [`redirect`].
unsafe fn missing_but_virtual_dir(raw: *const u16) -> bool {
    // ERROR_FILE_NOT_FOUND and ERROR_PATH_NOT_FOUND. Anything else (access
    // denied, a sharing violation) is a real answer and stays one.
    let error = GetLastError();

    if error != 2 && error != 3 {
        return false;
    }

    let Some(tree) = TREE.get() else {
        return false;
    };

    if raw.is_null() {
        return false;
    }

    match from_wide(raw) {
        Some(path) => tree.is_virtual_dir_absolute(&path),
        None => false,
    }
}

/// The real path for a virtual one, or `None` to pass through.
///
/// # Safety
///
/// `raw` must be a NUL-terminated wide string or null, which is what the API
/// contract of every function this is called from already guarantees.
unsafe fn redirect(raw: *const u16) -> Option<String> {
    let tree = TREE.get()?;

    if raw.is_null() {
        return None;
    }

    let asked = from_wide(raw)?;

    tree.resolve_absolute(&asked).map(str::to_string)
}

/// Read a NUL-terminated wide string, bounded.
///
/// # Safety
///
/// `raw` must be a NUL-terminated wide string. The bound is a second line: a
/// string that is not terminated within `MAX_PATH_LEN` is treated as not a path
/// rather than walked until it faults.
pub(crate) unsafe fn from_wide(raw: *const u16) -> Option<String> {
    let mut len = 0usize;

    while len < crate::tree::MAX_PATH_LEN {
        if *raw.add(len) == 0 {
            break;
        }

        len += 1;
    }

    if len == 0 || len >= crate::tree::MAX_PATH_LEN {
        return None;
    }

    let slice = std::slice::from_raw_parts(raw, len);

    OsString::from_wide(slice).into_string().ok()
}

pub(crate) fn to_wide(value: &str) -> Vec<u16> {
    let mut wide: Vec<u16> = std::ffi::OsStr::new(value).encode_wide().collect();

    wide.push(0);

    wide
}

// ----------------------------------------------------------- LoadLibrary

/// After a library load: patch whatever the loader just mapped.
///
/// The returned handle is not used. A `LOAD_LIBRARY_AS_DATAFILE` result is not
/// a loaded image and walking it as one would read garbage, and the loader may
/// have mapped dependencies that were never asked for by name. Re-walking the
/// process's real module list covers both, and the walk skips an import that
/// is already ours.
unsafe fn after_load(module: HMODULE) -> HMODULE {
    if !module.is_null() {
        patch_all_modules();
    }

    module
}

unsafe extern "system" fn hooked_load_library_a(name: *const u8) -> HMODULE {
    let saved = REAL_LOAD_LIBRARY_A.load(Ordering::Acquire);

    let real: LoadLibraryAFn = if saved == 0 {
        LoadLibraryA
    } else {
        std::mem::transmute::<usize, LoadLibraryAFn>(saved)
    };

    after_load(real(name))
}

unsafe extern "system" fn hooked_load_library_w(name: *const u16) -> HMODULE {
    let saved = REAL_LOAD_LIBRARY_W.load(Ordering::Acquire);

    let real: LoadLibraryWFn = if saved == 0 {
        LoadLibraryW
    } else {
        std::mem::transmute::<usize, LoadLibraryWFn>(saved)
    };

    after_load(real(name))
}

unsafe extern "system" fn hooked_load_library_ex_a(
    name: *const u8,
    file: HANDLE,
    flags: u32,
) -> HMODULE {
    let saved = REAL_LOAD_LIBRARY_EX_A.load(Ordering::Acquire);

    let real: LoadLibraryExAFn = if saved == 0 {
        LoadLibraryExA
    } else {
        std::mem::transmute::<usize, LoadLibraryExAFn>(saved)
    };

    after_load(real(name, file, flags))
}

unsafe extern "system" fn hooked_load_library_ex_w(
    name: *const u16,
    file: HANDLE,
    flags: u32,
) -> HMODULE {
    let saved = REAL_LOAD_LIBRARY_EX_W.load(Ordering::Acquire);

    let real: LoadLibraryExWFn = if saved == 0 {
        LoadLibraryExW
    } else {
        std::mem::transmute::<usize, LoadLibraryExWFn>(saved)
    };

    after_load(real(name, file, flags))
}

// ------------------------------------------------------------ IAT patching

/// One function to redirect: what to look for, what to put there, and where to
/// keep what was there.
struct Hook {
    name: &'static [u8],
    replacement: usize,
    saved: &'static AtomicUsize,
}

/// How many hooks [`table`] returns.
const HOOK_COUNT: usize = 13;

/// Every hook. Built on demand because a function's address is not a constant
/// expression; an array on the stack costs nothing and allocates nothing.
fn table() -> [Hook; HOOK_COUNT] {
    macro_rules! hook {
        ($name:literal, $function:expr, $saved:expr) => {
            Hook {
                name: concat!($name, "\0").as_bytes(),
                replacement: $function as usize,
                saved: $saved,
            }
        };
    }

    [
        hook!(
            "CreateFileW",
            hooked_create_file_w as CreateFileWFn,
            &REAL_CREATE_FILE_W
        ),
        hook!(
            "GetFileAttributesW",
            hooked_get_file_attributes_w as GetFileAttributesWFn,
            &REAL_GET_FILE_ATTRIBUTES_W
        ),
        hook!(
            "GetFileAttributesExW",
            hooked_get_file_attributes_ex_w as GetFileAttributesExWFn,
            &REAL_GET_FILE_ATTRIBUTES_EX_W
        ),
        hook!(
            "FindFirstFileW",
            find::hooked_find_first_file_w as find::FindFirstFileWFn,
            &find::REAL_FIND_FIRST_FILE_W
        ),
        hook!(
            "FindFirstFileExW",
            find::hooked_find_first_file_ex_w as find::FindFirstFileExWFn,
            &find::REAL_FIND_FIRST_FILE_EX_W
        ),
        hook!(
            "FindNextFileW",
            find::hooked_find_next_file_w as find::FindNextFileWFn,
            &find::REAL_FIND_NEXT_FILE_W
        ),
        hook!(
            "FindClose",
            find::hooked_find_close as find::FindCloseFn,
            &find::REAL_FIND_CLOSE
        ),
        hook!(
            "LoadLibraryA",
            hooked_load_library_a as LoadLibraryAFn,
            &REAL_LOAD_LIBRARY_A
        ),
        hook!(
            "LoadLibraryW",
            hooked_load_library_w as LoadLibraryWFn,
            &REAL_LOAD_LIBRARY_W
        ),
        hook!(
            "LoadLibraryExA",
            hooked_load_library_ex_a as LoadLibraryExAFn,
            &REAL_LOAD_LIBRARY_EX_A
        ),
        hook!(
            "LoadLibraryExW",
            hooked_load_library_ex_w as LoadLibraryExWFn,
            &REAL_LOAD_LIBRARY_EX_W
        ),
        hook!(
            "CreateProcessW",
            children::hooked_create_process_w as children::CreateProcessWFn,
            &children::REAL_CREATE_PROCESS_W
        ),
        hook!(
            "CreateProcessAsUserW",
            children::hooked_create_process_as_user_w as children::CreateProcessAsUserWFn,
            &children::REAL_CREATE_PROCESS_AS_USER_W
        ),
    ]
}

/// Does this import module name carry the Win32 file and process APIs?
///
/// `kernel32` historically; `kernelbase` and the `api-ms-win-*` API sets for
/// anything linked against a recent SDK (the UCRT imports its file calls from
/// `api-ms-win-core-file-*`); `advapi32` for `CreateProcessAsUserW`. Other DLLs
/// are not walked for matches, so a module exporting an unrelated function that
/// happens to be called `FindClose` is never patched.
///
/// # Safety
///
/// `raw` must be a NUL-terminated C string inside a mapped image.
unsafe fn is_win32_module(raw: *const u8) -> bool {
    [
        &b"kernel32.dll\0"[..],
        b"kernelbase.dll\0",
        b"advapi32.dll\0",
    ]
    .iter()
    .any(|expected| eq_ascii_ci(raw, expected))
        || starts_with_ascii_ci(raw, b"api-ms-win-")
}

/// Patch every module currently loaded in this process, returning how many
/// import slots were redirected.
///
/// Zero is reported honestly: a process where nothing imports these calls by
/// name is one this cannot help.
///
/// Safe to call again at any time, and called from every `LoadLibrary` hook.
/// An import that already points at its hook is left alone — recording it as
/// the "real" function would make the hook call itself forever.
fn patch_all_modules() -> usize {
    let hooks = table();

    let own = OWN_MODULE.load(Ordering::Acquire);

    // A stack buffer rather than a `Vec`: this runs inside `LoadLibrary`, on
    // whatever thread loaded something. A process with more than this many
    // modules gets the first of them patched; that is a limit worth stating
    // and not one a game reaches.
    let mut modules: [HMODULE; 1024] = [std::ptr::null_mut(); 1024];

    let mut needed: u32 = 0;

    let listed = unsafe {
        K32EnumProcessModules(
            GetCurrentProcess(),
            modules.as_mut_ptr(),
            std::mem::size_of_val(&modules) as u32,
            &mut needed,
        )
    };

    if listed == 0 {
        return 0;
    }

    let count = (needed as usize / std::mem::size_of::<HMODULE>()).min(modules.len());

    let mut patched = 0usize;

    for module in &modules[..count] {
        if module.is_null() || *module as usize == own {
            continue;
        }

        patched += unsafe { patch_module(*module, &hooks) };
    }

    patched
}

/// Redirect every matching import slot of one module.
///
/// The saved pointer is published BEFORE the patch that makes the hook
/// reachable. Patching first would open a window in which another thread
/// enters the hook and finds nothing to forward to.
///
/// # Safety
///
/// Walks the PE structures of a module the loader has already mapped. Every
/// header is checked for its magic before it is trusted, because a packed or
/// corrupted image is a real thing to meet in a game process and a wild read
/// here faults inside somebody else's executable.
unsafe fn patch_module(base: HMODULE, hooks: &[Hook]) -> usize {
    let bytes = base as *const u8;

    let dos = &*(bytes as *const IMAGE_DOS_HEADER);

    // `MZ`. A module whose header is not this is not a PE image and nothing
    // below it can be read.
    if dos.e_magic != 0x5A4D || dos.e_lfanew <= 0 || dos.e_lfanew > 0x0100_0000 {
        return 0;
    }

    let nt = &*(bytes.offset(dos.e_lfanew as isize) as *const IMAGE_NT_HEADERS64);

    // `PE\0\0`, and the 64-bit optional header: a 32-bit image cannot be in
    // this process, and reading one as 64-bit would misplace every field.
    if nt.Signature != 0x0000_4550 || nt.OptionalHeader.Magic != 0x20B {
        return 0;
    }

    let directory = nt.OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT as usize];

    if directory.VirtualAddress == 0 || directory.Size == 0 {
        return 0;
    }

    let mut patched = 0usize;

    let mut descriptor =
        bytes.add(directory.VirtualAddress as usize) as *const IMAGE_IMPORT_DESCRIPTOR;

    while (*descriptor).Name != 0 {
        let name = bytes.add((*descriptor).Name as usize);

        if is_win32_module(name) {
            // `OriginalFirstThunk` still holds the NAMES; `FirstThunk` is the
            // live pointer table the loader filled in. Both are needed: one to
            // find the entry, the other to patch it.
            let mut original = bytes.add((*descriptor).Anonymous.OriginalFirstThunk as usize)
                as *const IMAGE_THUNK_DATA64;

            let mut live = bytes.add((*descriptor).FirstThunk as usize) as *mut IMAGE_THUNK_DATA64;

            // A module bound at link time has no name table to walk.
            if (*descriptor).Anonymous.OriginalFirstThunk == 0 {
                original = live as *const IMAGE_THUNK_DATA64;
            }

            while (*original).u1.AddressOfData != 0 {
                // The high bit means "imported by ordinal", which carries no
                // name to compare.
                if (*original).u1.Ordinal & 0x8000_0000_0000_0000 == 0 {
                    // `IMAGE_IMPORT_BY_NAME` is a `u16` hint then the name.
                    let entry = bytes.add((*original).u1.AddressOfData as usize);
                    let import_name = entry.add(2);

                    for hook in hooks {
                        if eq_ascii_ci(import_name, hook.name) {
                            let slot = std::ptr::addr_of_mut!((*live).u1.Function) as *mut usize;

                            if patch_slot(slot, hook) {
                                patched += 1;
                            }

                            break;
                        }
                    }
                }

                original = original.add(1);
                live = live.add(1);
            }
        }

        descriptor = descriptor.add(1);
    }

    patched
}

/// Publish what is in one import slot, then overwrite it with the hook.
///
/// # Safety
///
/// `slot` must be an import-table entry produced by [`patch_module`].
unsafe fn patch_slot(slot: *mut usize, hook: &Hook) -> bool {
    let previous = std::ptr::read_volatile(slot);

    // Already ours (a second pass over the same module), or never resolved.
    if previous == hook.replacement || previous == 0 {
        return false;
    }

    // First writer wins. Two import tables can legitimately resolve one
    // function to two addresses (`kernel32` and its `kernelbase` forwarder);
    // the first seen is as good as the second, and keeping one means the
    // hook's own forwarding never changes under it.
    if hook.saved.load(Ordering::Acquire) == 0 {
        hook.saved.store(previous, Ordering::Release);
    }

    write_pointer(slot, hook.replacement)
}

/// Does `raw` begin with `prefix`, ignoring ASCII case?
///
/// # Safety
///
/// `raw` must be a NUL-terminated C string; the comparison stops at the first
/// NUL, so it never reads past the terminator.
unsafe fn starts_with_ascii_ci(raw: *const u8, prefix: &[u8]) -> bool {
    for (index, wanted) in prefix.iter().enumerate() {
        let actual = *raw.add(index);

        if actual == 0 || !actual.eq_ignore_ascii_case(wanted) {
            return false;
        }
    }

    true
}

/// Write one pointer into a page that is normally read-only.
///
/// # Safety
///
/// `slot` must be a valid, writable-after-`VirtualProtect` address inside a
/// mapped image — which is what the import-table walk above produces.
unsafe fn write_pointer(slot: *mut usize, value: usize) -> bool {
    let mut previous: PAGE_PROTECTION_FLAGS = 0;

    let size = std::mem::size_of::<usize>();

    if VirtualProtect(
        slot as *mut std::ffi::c_void,
        size,
        PAGE_READWRITE,
        &mut previous,
    ) == 0
    {
        return false;
    }

    *slot = value;

    // Put the protection back. Leaving an import table writable is a gift to
    // anything else in the process that wants to hook the same function.
    let mut ignored: PAGE_PROTECTION_FLAGS = 0;

    VirtualProtect(slot as *mut std::ffi::c_void, size, previous, &mut ignored);

    true
}

/// Compare a NUL-terminated C string with a NUL-terminated literal, ignoring
/// case.
///
/// # Safety
///
/// `raw` must be NUL-terminated. Bounded at 256 bytes, which is longer than any
/// module or export name Windows permits.
unsafe fn eq_ascii_ci(raw: *const u8, expected: &[u8]) -> bool {
    for (index, wanted) in expected.iter().enumerate() {
        if index > 256 {
            return false;
        }

        let actual = *raw.add(index);

        if actual.eq_ignore_ascii_case(wanted) {
            continue;
        }

        return false;
    }

    true
}

/// Keeps the linker from discarding a function nothing in Rust calls.
///
/// The hooks are only ever reached through a pointer written into somebody
/// else's import table, which the linker cannot see.
#[used]
static KEEP: [FARPROC; 0] = [];
