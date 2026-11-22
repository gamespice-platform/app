//! **Virtual filesystem deployment** — the strategy where the game folder is
//! never touched at all.
//!
//! Mod Organizer's approach, and the only one that leaves a game directory
//! byte-identical: instead of putting files there, the mods stay in staging and
//! the game is *told* they are there, by hooking the file APIs it uses to look.
//!
//! ```text
//!   app                              game process
//!   ───                              ────────────
//!   build the merged tree
//!   publish it            ──shm──▶
//!   launch suspended
//!   inject the hook DLL   ──────▶    DllMain → read the tree
//!                                    patch CreateFileW / GetFileAttributesW
//!   resume                ──────▶    game runs; every open of a virtual path
//!                                    is answered from staging
//! ```
//!
//! HOW FINISHED THIS IS
//! --------------------
//! Two halves, at very different levels of confidence, and it matters which is
//! which. This strategy is **EXPERIMENTAL**: on by default in the build
//! (`usvfs-hooks`), offered for Minecraft, but never the default strategy.
//!
//! | Half | State |
//! | --- | --- |
//! | [`tree`] - the merged view, resolution, listings, case rules | **Tested.** Runs on every platform, including the directory-listing merge |
//! | [`shm`] - publishing and reading the blob, atomically | **Tested.** Including every malformed input |
//! | [`env`] - rebuilding a child's environment block | **Tested.** Pure, on every platform |
//! | [`hooks`] - IAT patching, `CreateFileW`, attribute queries, `FindFirstFile`/`FindNextFile`, `CreateProcess` | **Type-checked and clippy-clean against the Windows target. Never run against a game.** |
//! | [`inject`] - suspended launch, remote `LoadLibrary` | **Type-checked. Argument quoting and environment building are tested; the launch itself is not.** |
//!
//! The hooks were written and checked on Linux with a cross-compiling target.
//! Nobody has yet watched them run in a Windows process, so the first real
//! launch is the first test. That is why the strategy is labelled experimental
//! and why `direct` stays the default.
//!
//! The `usvfs-hooks` feature is on by default. Without it (or off Windows)
//! `Strategy::Usvfs` refuses at deploy time and at launch time, in
//! `gamespice-app`'s `spawn` module. The gate is checked in both places on
//! purpose: a build where only one was on could publish a tree no launch would
//! carry, or inject with nothing to inject.
//!
//! CHILD PROCESSES
//! ---------------
//! `CreateProcessW` and `CreateProcessAsUserW` are hooked so a launcher's
//! helper processes and a wrapper's JVM are hooked too. The DLL is only ever
//! injected into a process the hooked process just created itself, suspended,
//! through the handle `CreateProcess` returned: there is no path that takes an
//! arbitrary process id. 32-bit children are skipped (a 64-bit DLL cannot load
//! there). `CreateProcessA`, `CreateProcessWithLogonW` and
//! `CreateProcessWithTokenW` are not hooked.
//!
//! //! WHAT IT CANNOT DO, EVEN WHEN IT WORKS
//! -------------------------------------
//!   * **Calls through `GetProcAddress` are not redirected**, because the
//!     pointer never came from an import table.
//!   * **Direct `ntdll` syscalls are not redirected.** Some engines and most
//!     anti-tamper layers do exactly this.
//!   * **Every kernel-level anti-cheat treats injection as an attack**, and
//!     correctly. A game declaring `antiCheat: "kernel"` is refused this
//!     strategy before it reaches here.
//!   * **Writes are not redirected.** A game creating a file in a virtualised
//!     directory writes it to the real one — see [`hooks`] for why that is a
//!     decision rather than a gap.
//!   * **Staged files are not copy-on-write.** A game that opens a virtual file
//!     for writing writes to the staged file itself.
//!   * **Virtual-only directories report zero timestamps**, and 8.3 short names
//!     are empty for virtual entries.
//!   * **Only the first 1024 loaded modules are patched**, and only imports by
//!     name from kernel32, kernelbase, advapi32 and `api-ms-win-*`.
//!
//! [`Strategy::Usvfs`]: https://docs.rs/tmc-core/latest/tmc_core/deploy/enum.Strategy.html

pub mod env;
pub mod shm;
pub mod tree;

#[cfg(windows)]
pub mod hooks;
#[cfg(windows)]
pub mod inject;

pub use shm::{publish, read, Published, ENV_BLOB, ENV_ROOT};
pub use tree::{Mapping, VirtualTree};

/// The DLL's entry point.
///
/// Installs the hooks on attach and does nothing else — no thread is spawned,
/// nothing is allocated beyond the tree, and every failure leaves the game
/// running completely unmodified rather than half-hooked.
///
/// `DllMain` runs under the loader lock, which is why the work here is a file
/// read and a table patch and not one instruction more. Anything that took a
/// lock, started a thread, or called into another DLL would be the classic
/// loader-lock deadlock, on a machine that is not this one.
///
/// # Safety
///
/// Called by the Windows loader with its own arguments.
#[cfg(windows)]
#[no_mangle]
pub unsafe extern "system" fn DllMain(
    _module: *mut std::ffi::c_void,
    reason: u32,
    _reserved: *mut std::ffi::c_void,
) -> i32 {
    /// `DLL_PROCESS_ATTACH`.
    const ATTACH: u32 = 1;

    if reason == ATTACH {
        // The return value is deliberately ignored: a process this DLL cannot
        // help is one it should stay quietly loaded in, not one it should fail
        // to load into — a failed `DllMain` terminates the process.
        let _ = hooks::install();
    }

    1
}

/// Build a virtual tree from a deployment's merge result.
///
/// The bridge between the deployment engine's own model and this one. Kept here
/// rather than in the engine so the engine needs no knowledge of virtual paths,
/// and so this crate can be tested without it.
///
/// `entries` arrives in ASCENDING priority order, which is the same order the
/// merge tree resolves in — a later entry wins.
pub fn tree_from(
    game_dir: &str,
    entries: impl IntoIterator<Item = (String, String)>,
) -> VirtualTree {
    let mut tree = VirtualTree::new(game_dir);

    for (virtual_path, real_path) in entries {
        tree.insert(&virtual_path, &real_path);
    }

    tree
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_is_built_from_a_deployments_entries() {
        let tree = tree_from(
            "C:/Games/X",
            [
                (
                    "mods/a.jar".to_string(),
                    "D:/staging/1/mods/a.jar".to_string(),
                ),
                (
                    "mods/b.jar".to_string(),
                    "D:/staging/2/mods/b.jar".to_string(),
                ),
            ],
        );

        assert_eq!(tree.len(), 2);
        assert_eq!(
            tree.resolve_absolute("C:/Games/X/mods/a.jar"),
            Some("D:/staging/1/mods/a.jar")
        );
        assert!(tree.extra_entries("mods").contains("b.jar"));
    }

    /// Ascending priority in, later wins — the same rule the merge tree
    /// resolves by, so the two cannot disagree about which mod won.
    #[test]
    fn the_last_entry_for_a_path_wins() {
        let tree = tree_from(
            "C:/Games/X",
            [
                ("mods/a.jar".to_string(), "D:/low/a.jar".to_string()),
                ("mods/a.jar".to_string(), "D:/high/a.jar".to_string()),
            ],
        );

        assert_eq!(tree.resolve("mods/a.jar"), Some("D:/high/a.jar"));
    }
}
