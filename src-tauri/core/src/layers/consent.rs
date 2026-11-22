//! Consent to a plan (docs/loaders-and-versions.md, "Deploy consumption",
//! phase 2).
//!
//! A plan that runs an installer program, downloads a runtime, or carries a
//! manifest consent (the Minecraft EULA for a server) is deployed only after
//! the user agreed to that exact plan. The agreement is stored per sandbox and
//! plan hash (`sandbox_consent`), so a new manifest revision, another build or
//! another runtime changes the hash and asks again.
//!
//! Only the newest accepted plan stands for a sandbox: recording consent drops
//! the sandbox's other agreements ([`LibraryDb::sandbox_record_consent`]), so
//! an older plan that comes back (a stale cached plan, a rolled-back build)
//! asks again rather than replaying the agreement given to it once.

use crate::error::{AppError, AppResult};
use crate::layers::plan::InstallPlan;
use crate::library::db::LibraryDb;

/// The plan asks for the user's agreement before it deploys.
pub fn needs_consent(plan: &InstallPlan) -> bool {
    !plan.consents.is_empty()
}

/// `Err(NeedsConsent(plan_hash))` unless the user agreed to this plan for
/// this sandbox.
pub fn check(db: &LibraryDb, sandbox_id: i64, plan: &InstallPlan) -> AppResult<()> {
    if !needs_consent(plan) {
        return Ok(());
    }

    let given = db.sandbox_consent(sandbox_id, &plan.plan_hash)?;
    let wanted: Vec<&str> = plan.consents.iter().map(|c| c.key.as_str()).collect();

    match given {
        Some(keys) if wanted.iter().all(|w| keys.iter().any(|k| k == w)) => Ok(()),
        _ => Err(AppError::NeedsConsent(plan.plan_hash.clone())),
    }
}

/// Record that the user agreed to `plan_hash`. The hash must be the one the
/// current plan has: a consent to a plan the user never saw is refused.
pub fn record(
    db: &LibraryDb,
    sandbox_id: i64,
    plan: &InstallPlan,
    plan_hash: &str,
) -> AppResult<()> {
    if plan.plan_hash != plan_hash {
        return Err(AppError::invalid(
            "The install plan changed since it was shown. Review it again.",
        ));
    }

    let keys: Vec<String> = plan.consents.iter().map(|c| c.key.clone()).collect();
    db.sandbox_record_consent(sandbox_id, plan_hash, &keys)
}
