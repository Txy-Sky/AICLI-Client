use crate::deeplink::PendingDeepLinkEntry;

/// Read the deep link waiting for the frontend without clearing it (see `PendingDeepLink`)
#[tauri::command]
pub fn peek_pending_deeplink() -> Option<PendingDeepLinkEntry> {
    crate::deeplink::peek_pending_deeplink()
}

/// Clear the pending deep link once the frontend has handled it; ignored if `id` is stale
#[tauri::command]
pub fn ack_pending_deeplink(id: u64) {
    crate::deeplink::ack_pending_deeplink(id);
}
