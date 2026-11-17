//! Updates from the GitHub releases: the UI checks a moment after the start
//! and the user chooses when to install. Updates are signed; only ones signed
//! with the key whose public half is in tauri.conf.json are installed.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

use crate::backend::CmdResult;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    notes: Option<String>,
}

/// The newer version on offer, if any.
#[tauri::command]
pub async fn check_update(app: AppHandle) -> CmdResult<Option<UpdateInfo>> {
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    Ok(update.map(|u| UpdateInfo { version: u.version, notes: u.body }))
}

/// Downloads and installs the newer version, telling the UI how far the
/// download is (`update-progress`: [bytes, total bytes or null]), then
/// restarts. On Windows the installer closes Coolmus and starts it again.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> CmdResult<()> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else { return Ok(()) };
    let mut done = 0;
    update
        .download_and_install(
            |chunk, total| {
                done += chunk;
                let _ = app.emit("update-progress", (done, total));
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    app.restart()
}
