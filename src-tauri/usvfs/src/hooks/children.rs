//! Child-process propagation: `CreateProcessW` and `CreateProcessAsUserW`.
//!
//! Minecraft's launcher starts the game through a wrapper, and mods start
//! helper processes. A hook that stopped at the first process would leave
//! every one of those seeing the real directories. These hooks carry it down.
//!
//! SAFETY STANCE
//! -------------
//! The DLL is only ever put into a process this process JUST created through
//! one of these two calls: a child of the supervised tree, suspended before
//! its first instruction, in a handle the real `CreateProcess` handed back.
//! There is no code path that takes a process id from anywhere else, and no
//! `OpenProcess`.
//!
//! THE SEQUENCE
//! ------------
//!   1. add `CREATE_SUSPENDED` if the caller did not ask for it;
//!   2. if the caller supplied an environment block, rebuild it with the two
//!      variables the DLL reads at attach (a null block inherits ours, which
//!      already has them);
//!   3. call the real function;
//!   4. unless the child is 32-bit (a 64-bit DLL cannot load there), inject;
//!   5. resume the main thread, but only if the caller did not ask for the
//!      child suspended. A failed injection still resumes: a launch that
//!      silently hangs is worse than a child that runs unhooked.
//!
//! Type-checked only: this has never run on Windows.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_ACP};
use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
use windows_sys::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, CreateProcessW, IsWow64Process, ResumeThread, CREATE_SUSPENDED,
    CREATE_UNICODE_ENVIRONMENT, PROCESS_INFORMATION, STARTUPINFOW,
};

use super::to_wide;
use crate::env::{parse_wide, with_additions};
use crate::shm::{ENV_BLOB, ENV_ROOT};

pub(crate) type CreateProcessWFn = unsafe extern "system" fn(
    *const u16,
    *mut u16,
    *const SECURITY_ATTRIBUTES,
    *const SECURITY_ATTRIBUTES,
    i32,
    u32,
    *const c_void,
    *const u16,
    *const STARTUPINFOW,
    *mut PROCESS_INFORMATION,
) -> i32;

pub(crate) type CreateProcessAsUserWFn = unsafe extern "system" fn(
    HANDLE,
    *const u16,
    *mut u16,
    *const SECURITY_ATTRIBUTES,
    *const SECURITY_ATTRIBUTES,
    i32,
    u32,
    *const c_void,
    *const u16,
    *const STARTUPINFOW,
    *mut PROCESS_INFORMATION,
) -> i32;

pub(crate) static REAL_CREATE_PROCESS_W: AtomicUsize = AtomicUsize::new(0);
pub(crate) static REAL_CREATE_PROCESS_AS_USER_W: AtomicUsize = AtomicUsize::new(0);

/// Children whose injection failed, for diagnosis. Never read by the hook.
pub(crate) static INJECT_FAILURES: AtomicU32 = AtomicU32::new(0);
/// Children skipped because they are 32-bit.
pub(crate) static SKIPPED_WOW64: AtomicU32 = AtomicU32::new(0);

/// Our `CreateProcessW`.
///
/// # Safety
///
/// Called by the game with `CreateProcessW`'s contract.
pub(crate) unsafe extern "system" fn hooked_create_process_w(
    application: *const u16,
    command: *mut u16,
    process_attributes: *const SECURITY_ATTRIBUTES,
    thread_attributes: *const SECURITY_ATTRIBUTES,
    inherit: i32,
    flags: u32,
    environment: *const c_void,
    directory: *const u16,
    startup: *const STARTUPINFOW,
    info: *mut PROCESS_INFORMATION,
) -> i32 {
    let real: CreateProcessWFn = match REAL_CREATE_PROCESS_W.load(Ordering::Acquire) {
        0 => {
            std::mem::transmute::<*const c_void, CreateProcessWFn>(CreateProcessW as *const c_void)
        }
        saved => std::mem::transmute::<usize, CreateProcessWFn>(saved),
    };

    let mut prepared = Prepared::new(flags, environment, info);

    let ok = real(
        application,
        command,
        process_attributes,
        thread_attributes,
        inherit,
        prepared.flags,
        prepared.environment(),
        directory,
        startup,
        prepared.info,
    );

    prepared.finish(ok != 0);

    ok
}

/// Our `CreateProcessAsUserW`.
///
/// # Safety
///
/// Called by the game with `CreateProcessAsUserW`'s contract.
pub(crate) unsafe extern "system" fn hooked_create_process_as_user_w(
    token: HANDLE,
    application: *const u16,
    command: *mut u16,
    process_attributes: *const SECURITY_ATTRIBUTES,
    thread_attributes: *const SECURITY_ATTRIBUTES,
    inherit: i32,
    flags: u32,
    environment: *const c_void,
    directory: *const u16,
    startup: *const STARTUPINFOW,
    info: *mut PROCESS_INFORMATION,
) -> i32 {
    let real: CreateProcessAsUserWFn = match REAL_CREATE_PROCESS_AS_USER_W.load(Ordering::Acquire) {
        0 => std::mem::transmute::<*const c_void, CreateProcessAsUserWFn>(
            CreateProcessAsUserW as *const c_void,
        ),
        saved => std::mem::transmute::<usize, CreateProcessAsUserWFn>(saved),
    };

    let mut prepared = Prepared::new(flags, environment, info);

    let ok = real(
        token,
        application,
        command,
        process_attributes,
        thread_attributes,
        inherit,
        prepared.flags,
        prepared.environment(),
        directory,
        startup,
        prepared.info,
    );

    prepared.finish(ok != 0);

    ok
}

/// The rewritten arguments for one call, and what to do afterwards.
struct Prepared {
    flags: u32,
    caller_wanted_suspended: bool,
    /// The caller's `PROCESS_INFORMATION`, or ours when they passed none.
    info: *mut PROCESS_INFORMATION,
    caller_gave_info: bool,
    own_info: PROCESS_INFORMATION,
    original_environment: *const c_void,
    rebuilt: Option<Vec<u16>>,
    active: bool,
}

impl Prepared {
    /// # Safety
    ///
    /// `environment` must be null or a valid environment block, as for the real
    /// call.
    unsafe fn new(flags: u32, environment: *const c_void, info: *mut PROCESS_INFORMATION) -> Self {
        // Nothing to propagate when this process has no tree (the DLL attached
        // but failed to load one), or no DLL path to inject.
        let active = super::tree().is_some() && super::own_path().is_some();

        let mut prepared = Prepared {
            flags,
            caller_wanted_suspended: flags & CREATE_SUSPENDED != 0,
            info,
            caller_gave_info: !info.is_null(),
            own_info: std::mem::zeroed(),
            original_environment: environment,
            rebuilt: None,
            active,
        };

        if !active {
            return prepared;
        }

        prepared.flags |= CREATE_SUSPENDED;

        if !prepared.caller_gave_info {
            prepared.info = &mut prepared.own_info;
        }

        if !environment.is_null() {
            prepared.rebuilt = rebuild_environment(environment, flags);

            if prepared.rebuilt.is_some() {
                prepared.flags |= CREATE_UNICODE_ENVIRONMENT;
            }
        }

        prepared
    }

    fn environment(&self) -> *const c_void {
        match &self.rebuilt {
            Some(block) => block.as_ptr() as *const c_void,
            None => self.original_environment,
        }
    }

    /// Inject and resume after the real call.
    ///
    /// # Safety
    ///
    /// Uses the handles the real call just produced.
    unsafe fn finish(&mut self, created: bool) {
        if !self.active || !created {
            return;
        }

        // `info` points at the caller's struct or ours; the real call has
        // filled it in.
        let info = &*self.info;

        let mut wow64 = 0i32;

        let is_32_bit = IsWow64Process(info.hProcess, &mut wow64) != 0 && wow64 != 0;

        if is_32_bit {
            SKIPPED_WOW64.fetch_add(1, Ordering::Relaxed);
        } else if let Some(dll) = super::own_path() {
            if let Err(error) = crate::inject::inject(info.hProcess, dll) {
                INJECT_FAILURES.fetch_add(1, Ordering::Relaxed);

                let note = to_wide(&format!("tmc-usvfs: child injection failed: {error}"));

                OutputDebugStringW(note.as_ptr());
            }
        }

        if !self.caller_wanted_suspended {
            ResumeThread(info.hThread);
        }

        // Handles the caller never asked for are ours to close.
        if !self.caller_gave_info {
            CloseHandle(info.hThread);
            CloseHandle(info.hProcess);
        }
    }
}

/// The caller's block with the hook variables present, as a wide block.
/// `None` leaves the caller's block alone (the variables are not set in this
/// process, so there is nothing to add).
///
/// # Safety
///
/// `block` must be a valid environment block: wide if `flags` has
/// `CREATE_UNICODE_ENVIRONMENT`, ANSI otherwise, double-NUL terminated.
unsafe fn rebuild_environment(block: *const c_void, flags: u32) -> Option<Vec<u16>> {
    let blob = std::env::var(ENV_BLOB).ok()?;
    let root = std::env::var(ENV_ROOT).ok()?;

    let entries = if flags & CREATE_UNICODE_ENVIRONMENT != 0 {
        let units = block as *const u16;

        let mut len = 0usize;

        // Ends at the first empty entry, i.e. two NULs in a row.
        while !(*units.add(len) == 0 && (len == 0 || *units.add(len - 1) == 0)) {
            len += 1;
        }

        parse_wide(std::slice::from_raw_parts(units, len + 1))
    } else {
        let bytes = block as *const u8;

        let mut len = 0usize;

        while !(*bytes.add(len) == 0 && (len == 0 || *bytes.add(len - 1) == 0)) {
            len += 1;
        }

        let ansi = std::slice::from_raw_parts(bytes, len + 1);

        let needed = MultiByteToWideChar(
            CP_ACP,
            0,
            ansi.as_ptr(),
            ansi.len() as i32,
            std::ptr::null_mut(),
            0,
        );

        if needed <= 0 {
            return None;
        }

        let mut wide = vec![0u16; needed as usize];

        let written = MultiByteToWideChar(
            CP_ACP,
            0,
            ansi.as_ptr(),
            ansi.len() as i32,
            wide.as_mut_ptr(),
            needed,
        );

        if written <= 0 {
            return None;
        }

        parse_wide(&wide[..written as usize])
    };

    Some(with_additions(
        entries,
        &[(ENV_BLOB, blob.as_str()), (ENV_ROOT, root.as_str())],
    ))
}
