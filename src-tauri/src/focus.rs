use tauri::command;

use crate::focusers;

#[derive(serde::Deserialize)]
pub struct FocusRequest {
    pub kind: String,
    pub focus_id: String,
    pub outer_id: String,
}

#[command]
pub fn focus_terminal(req: FocusRequest) -> Result<(), String> {
    log::debug!(
        "focus_terminal: kind_hash={} focus_hash={}",
        crate::privacy::fingerprint(&req.kind),
        crate::privacy::fingerprint(&req.focus_id)
    );
    focusers::dispatch(&req.kind, &req.focus_id, &req.outer_id)
}
