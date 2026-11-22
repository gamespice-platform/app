//! `deploy://plan-step`: one step of a deploy's install plan changed state
//! (docs/loaders-and-versions.md, "Deploy consumption", phase 4).
//!
//! The install executor reports each step it starts, finishes, skips or fails
//! by its index in the layer's `install_steps`; the layer build maps that to
//! the index the preview shows (`PlannedLayer::steps`, which lists only the
//! steps that apply) and the deploy names the sandbox. The app forwards the
//! event to the deploy-progress dialog unchanged.

use std::sync::Arc;

use serde::Serialize;

/// What happened to a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    Running,
    Done,
    Skipped,
    Failed,
}

/// The payload of `deploy://plan-step`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStepEvent {
    pub sandbox_id: i64,
    pub plan_hash: String,
    /// The layer's manifest key.
    pub layer: String,
    /// The step's index in the layer's previewed steps.
    pub index: usize,
    /// How many steps the layer's preview lists.
    pub total: usize,
    pub status: StepStatus,
    pub bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    /// The end of the installer's output (or the error) of a failed step.
    pub log_tail: Option<String>,
}

/// Where plan-step events go.
pub type PlanStepSink = Arc<dyn Fn(PlanStepEvent) + Send + Sync>;

/// The executor's side: `(index in install_steps, status, log tail)`.
pub type StepSink = Arc<dyn Fn(usize, StepStatus, Option<String>) + Send + Sync>;

/// How much of an installer log a failed step carries.
pub const LOG_TAIL_LINES: usize = 40;

/// The last [`LOG_TAIL_LINES`] lines of `text`.
pub fn tail(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let from = lines.len().saturating_sub(LOG_TAIL_LINES);
    lines[from..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_event_serialises_as_the_ui_schema_reads_it() {
        let e = PlanStepEvent {
            sandbox_id: 3,
            plan_hash: "h".into(),
            layer: "minecraft/forge".into(),
            index: 1,
            total: 4,
            status: StepStatus::Failed,
            bytes: None,
            total_bytes: None,
            log_tail: Some("boom".into()),
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "sandboxId": 3, "planHash": "h", "layer": "minecraft/forge",
                "index": 1, "total": 4, "status": "failed",
                "bytes": null, "totalBytes": null, "logTail": "boom"
            })
        );
    }

    #[test]
    fn tail_keeps_the_last_lines() {
        let text: String = (0..100).map(|i| format!("{i}\n")).collect();
        let t = tail(&text);
        assert_eq!(t.lines().count(), LOG_TAIL_LINES);
        assert!(t.ends_with("99"));
    }
}
