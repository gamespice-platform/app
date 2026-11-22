//! Which app owns `nxm://`, `curseforge://`, `modrinth://`, `ror2mm://`?
//!
//! Reading is free; writing happens only on an explicit call and never takes a
//! scheme another app holds unless `override_existing` is set. Windows reads
//! the registry (`reg query`), Linux asks xdg-mime. macOS is not supported:
//! its handler table is per-app Info.plist plus Launch Services, with no
//! supported way to read or reassign it from here, so it reports `Unknown`.

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

/// The schemes a user can hand to this app.
pub const MANAGED: [&str; 5] = ["nxm", "curseforge", "modrinth", "ror2mm", "prismlauncher"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum SchemeStatus {
    /// Some app owns it. `by` is its command or desktop file; `ours` when it is this app.
    Registered {
        by: String,
        ours: bool,
    },
    Free,
    Unknown,
}

/// Runs a program and returns stdout on success, `None` when it exited non-zero
/// or could not start. Behind a trait so tests never spawn anything.
pub trait CommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String>;
}

pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> Option<String> {
        let out = std::process::Command::new(program)
            .args(args)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Linux,
    Mac,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Linux
        }
    }
}

fn check(scheme: &str) -> AppResult<()> {
    if MANAGED.contains(&scheme) {
        Ok(())
    } else {
        Err(AppError::invalid(format!(
            "{scheme}:// is not a scheme this app manages"
        )))
    }
}

/// `own` is this app's command or desktop-file name, to recognise ourselves.
pub fn status(run: &dyn CommandRunner, os: Os, scheme: &str, own: &str) -> AppResult<SchemeStatus> {
    check(scheme)?;
    Ok(match os {
        Os::Mac => SchemeStatus::Unknown,
        Os::Windows => {
            let key = format!("HKCU\\Software\\Classes\\{scheme}\\shell\\open\\command");
            match run.run("reg", &["query", &key, "/ve"]) {
                None => SchemeStatus::Free,
                Some(out) => match parse_reg_default(&out) {
                    Some(cmd) => registered(cmd, own),
                    None => SchemeStatus::Unknown,
                },
            }
        }
        Os::Linux => {
            let mime = format!("x-scheme-handler/{scheme}");
            match run.run("xdg-mime", &["query", "default", &mime]) {
                None => SchemeStatus::Unknown,
                Some(out) => {
                    let d = out.trim();
                    if d.is_empty() {
                        SchemeStatus::Free
                    } else {
                        registered(d.to_string(), own)
                    }
                }
            }
        }
    })
}

fn registered(by: String, own: &str) -> SchemeStatus {
    let ours = !own.is_empty() && by.to_ascii_lowercase().contains(&own.to_ascii_lowercase());
    SchemeStatus::Registered { by, ours }
}

/// The `(Default)` value from `reg query ... /ve` output.
fn parse_reg_default(out: &str) -> Option<String> {
    out.lines().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_prefix("(Default)")?.trim_start();
        let rest = rest.strip_prefix("REG_SZ")?.trim();
        (!rest.is_empty()).then(|| rest.to_string())
    })
}

/// Claim a scheme for this app. `exe` is the path to register (Windows) and
/// `desktop` the desktop-file name (Linux). Refuses when another app holds it
/// and `override_existing` is false.
pub fn set(
    run: &dyn CommandRunner,
    os: Os,
    scheme: &str,
    own: &str,
    exe: &str,
    desktop: &str,
    override_existing: bool,
) -> AppResult<SchemeStatus> {
    if os == Os::Mac {
        return Err(AppError::invalid("macOS does not let an app reassign link handlers from here; use the other app's settings"));
    }
    if let SchemeStatus::Registered { by, ours: false } = status(run, os, scheme, own)? {
        if !override_existing {
            return Err(AppError::invalid(format!(
                "{scheme}:// is already handled by {by}; confirm to replace it"
            )));
        }
    }
    let ok = match os {
        Os::Windows => {
            let base = format!("HKCU\\Software\\Classes\\{scheme}");
            let cmd = format!("\"{exe}\" \"%1\"");
            run.run(
                "reg",
                &["add", &base, "/ve", "/d", &format!("URL:{scheme}"), "/f"],
            )
            .is_some()
                && run
                    .run("reg", &["add", &base, "/v", "URL Protocol", "/d", "", "/f"])
                    .is_some()
                && run
                    .run(
                        "reg",
                        &[
                            "add",
                            &format!("{base}\\shell\\open\\command"),
                            "/ve",
                            "/d",
                            &cmd,
                            "/f",
                        ],
                    )
                    .is_some()
        }
        _ => run
            .run(
                "xdg-mime",
                &["default", desktop, &format!("x-scheme-handler/{scheme}")],
            )
            .is_some(),
    };
    if !ok {
        return Err(AppError::invalid(format!("could not register {scheme}://")));
    }
    status(run, os, scheme, own)
}

/// Give a scheme back, only if it is ours.
pub fn unset(run: &dyn CommandRunner, os: Os, scheme: &str, own: &str) -> AppResult<SchemeStatus> {
    match status(run, os, scheme, own)? {
        SchemeStatus::Registered { ours: true, .. } => {}
        SchemeStatus::Registered { by, .. } => {
            return Err(AppError::invalid(format!(
                "{scheme}:// belongs to {by}, not this app; leaving it"
            )))
        }
        other => return Ok(other),
    }
    if os == Os::Windows {
        run.run(
            "reg",
            &[
                "delete",
                &format!("HKCU\\Software\\Classes\\{scheme}"),
                "/f",
            ],
        );
        status(run, os, scheme, own)
    } else {
        // xdg-mime has no unset; the default is dropped by removing the mimeapps entry,
        // which only the user's desktop settings should do.
        Err(AppError::invalid(
            "on Linux, release the link in your desktop's default-apps settings",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Mock {
        answers: RefCell<Vec<(String, Option<String>)>>,
        log: RefCell<Vec<String>>,
        handler: RefCell<Option<String>>,
    }
    impl Mock {
        fn new(handler: Option<&str>) -> Self {
            Self {
                answers: RefCell::new(vec![]),
                log: RefCell::new(vec![]),
                handler: RefCell::new(handler.map(Into::into)),
            }
        }
    }
    impl CommandRunner for Mock {
        fn run(&self, program: &str, args: &[&str]) -> Option<String> {
            self.log
                .borrow_mut()
                .push(format!("{program} {}", args.join(" ")));
            let _ = &self.answers;
            match (program, args.first().copied()) {
                ("xdg-mime", Some("query")) => {
                    Some(self.handler.borrow().clone().unwrap_or_default() + "\n")
                }
                ("xdg-mime", Some("default")) => {
                    *self.handler.borrow_mut() = Some(args[1].to_string());
                    Some(String::new())
                }
                ("reg", Some("query")) => self
                    .handler
                    .borrow()
                    .as_ref()
                    .map(|h| format!("\r\nHKEY\\x\r\n    (Default)    REG_SZ    {h}\r\n")),
                ("reg", Some("add"))
                    if args.contains(&"command") || args.iter().any(|a| a.ends_with("command")) =>
                {
                    *self.handler.borrow_mut() =
                        Some(args[args.iter().position(|a| *a == "/d").unwrap() + 1].to_string());
                    Some(String::new())
                }
                ("reg", Some("add")) => Some(String::new()),
                ("reg", Some("delete")) => {
                    *self.handler.borrow_mut() = None;
                    Some(String::new())
                }
                _ => None,
            }
        }
    }

    #[test]
    fn reads_linux_and_windows_state() {
        assert_eq!(
            status(&Mock::new(None), Os::Linux, "nxm", "tmc").unwrap(),
            SchemeStatus::Free
        );
        assert_eq!(
            status(&Mock::new(Some("vortex.desktop")), Os::Linux, "nxm", "tmc").unwrap(),
            SchemeStatus::Registered {
                by: "vortex.desktop".into(),
                ours: false
            }
        );
        assert_eq!(
            status(&Mock::new(None), Os::Windows, "nxm", "tmc").unwrap(),
            SchemeStatus::Free
        );
        assert_eq!(
            status(
                &Mock::new(Some("\"C:\\Tmc\\tmc.exe\" \"%1\"")),
                Os::Windows,
                "nxm",
                "tmc.exe"
            )
            .unwrap(),
            SchemeStatus::Registered {
                by: "\"C:\\Tmc\\tmc.exe\" \"%1\"".into(),
                ours: true
            }
        );
        assert_eq!(
            status(&Mock::new(None), Os::Mac, "nxm", "tmc").unwrap(),
            SchemeStatus::Unknown
        );
    }

    #[test]
    fn unmanaged_schemes_are_refused() {
        assert!(status(&Mock::new(None), Os::Linux, "http", "tmc").is_err());
    }

    #[test]
    fn set_takes_a_free_scheme() {
        let m = Mock::new(None);
        let s = set(&m, Os::Linux, "nxm", "tmc", "", "tmc.desktop", false).unwrap();
        assert_eq!(
            s,
            SchemeStatus::Registered {
                by: "tmc.desktop".into(),
                ours: true
            }
        );
    }

    #[test]
    fn set_never_overwrites_without_override() {
        let m = Mock::new(Some("vortex.desktop"));
        assert!(set(&m, Os::Linux, "nxm", "tmc", "", "tmc.desktop", false).is_err());
        assert!(
            !m.log
                .borrow()
                .iter()
                .any(|l| l.starts_with("xdg-mime default")),
            "nothing written"
        );
        let s = set(&m, Os::Linux, "nxm", "tmc", "", "tmc.desktop", true).unwrap();
        assert_eq!(
            s,
            SchemeStatus::Registered {
                by: "tmc.desktop".into(),
                ours: true
            }
        );
    }

    #[test]
    fn windows_set_and_unset_only_touch_our_own() {
        let m = Mock::new(None);
        let s = set(
            &m,
            Os::Windows,
            "nxm",
            "tmc.exe",
            "C:\\Tmc\\tmc.exe",
            "",
            false,
        )
        .unwrap();
        assert!(matches!(s, SchemeStatus::Registered { ours: true, .. }));
        assert_eq!(
            unset(&m, Os::Windows, "nxm", "tmc.exe").unwrap(),
            SchemeStatus::Free
        );
        let other = Mock::new(Some("vortex.exe \"%1\""));
        assert!(unset(&other, Os::Windows, "nxm", "tmc.exe").is_err());
        assert!(!other
            .log
            .borrow()
            .iter()
            .any(|l| l.starts_with("reg delete")));
    }

    #[test]
    fn mac_cannot_set() {
        assert!(set(&Mock::new(None), Os::Mac, "nxm", "tmc", "", "", true).is_err());
    }
}
