//! Tauri commands for user-authored skills.

use crate::error::IpcResult;
use crate::settings::Settings;
use crate::skills::{self, Skill};

#[tauri::command]
pub async fn skills_list() -> IpcResult<Vec<Skill>> {
    skills::list().await.map_err(|e| e.to_string().into())
}

#[derive(Debug, serde::Deserialize)]
pub struct SkillInput {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub body: String,
}

#[tauri::command]
pub async fn skills_create(input: SkillInput) -> IpcResult<Skill> {
    skills::create(
        &input.id,
        &input.name,
        input.description.as_deref(),
        &input.body,
    )
    .await
    .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn skills_update(input: SkillInput) -> IpcResult<Skill> {
    skills::update(
        &input.id,
        &input.name,
        input.description.as_deref(),
        &input.body,
    )
    .await
    .map_err(|e| e.to_string().into())
}

#[tauri::command]
pub async fn skills_delete(id: String) -> IpcResult<()> {
    skills::delete(&id).await.map_err(|e| e.to_string().into())
}

/// Toggle a skill's enabled state in `settings.skills_enabled`. The runner
/// reads this list every turn so changes take effect on the next message.
#[tauri::command]
pub async fn skills_set_enabled(id: String, enabled: bool) -> IpcResult<()> {
    let mut s = Settings::load().await.map_err(|e| e.to_string())?;
    let has = s.skills_enabled.iter().any(|x| x == &id);
    if enabled && !has {
        s.skills_enabled.push(id);
    } else if !enabled && has {
        s.skills_enabled.retain(|x| x != &id);
    } else {
        return Ok(());
    }
    s.save().await.map_err(|e| e.to_string().into())
}

/// Return the raw `SKILL.md` text. Used by the editor in the Skills page.
#[tauri::command]
pub async fn skills_read_source(id: String) -> IpcResult<String> {
    let s = skills::load_with_body(&id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(tokio::fs::read_to_string(&s.meta.path)
        .await
        .map_err(|e| e.to_string())?)
}
