//! Attachment hooks are implemented in gb-attachments-wrapper/gb-attachments-run.
//! REA server-context-residual.md RC01 establishes staged image preparation;
//! linking and the image-only journal placeholder are local integration choices.
empty_hooks!();

/// UI-safe attachment DTOs for submission projection, never raw image bytes.
pub fn for_message(_: &super::Context<'_>, _: &str, _: &str) -> Vec<serde_json::Value> {
    Vec::new()
}
