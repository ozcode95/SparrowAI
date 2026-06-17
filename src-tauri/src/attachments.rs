//! Per-conversation file attachments (images + documents).
//!
//! The frontend hands us a *source* path (from the OS file picker) plus the
//! conversation id; we copy the bytes into a stable location under
//! `~/.zero/attachments/<conv_id>/<uuid>.<ext>` and return the metadata the
//! chat store persists alongside the message.
//!
//! Runtime (the chat runner) re-reads the persisted copy when it builds the
//! multimodal request payload — that way the conversation is reproducible
//! even if the user later moves or deletes the original file.

use crate::paths;
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Hard cap on a single attachment. Keeps a stray multi-GB drop from
/// instantly OOMing the runner — the user gets a clear error instead.
pub const MAX_ATTACHMENT_BYTES: u64 = 32 * 1024 * 1024; // 32 MiB

/// Maximum bytes of a text document we'll inline verbatim into the prompt.
/// Anything bigger is truncated with a tail marker so the model still sees
/// the start of the doc + a note that it was clipped.
pub const MAX_INLINE_TEXT_BYTES: usize = 256 * 1024;

/// One persisted attachment record. Mirrors the frontend `Attachment`
/// interface in `src/stores/chat.ts`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    /// `"image"` | `"document"` | `"audio"`. We currently classify by mime
    /// prefix; unknown types fall through as `"document"`.
    pub kind: String,
    /// Absolute path on disk under `~/.zero/attachments/`. The runner reads
    /// this back when building the request.
    pub path: String,
    pub mime: String,
    pub bytes: u64,
    /// Original filename the user picked. Surfaced as `[file: ...]` in the
    /// inlined doc text and as the alt-text for image chips in the UI.
    #[serde(default)]
    pub name: String,
}

/// Copy `source` into the conversation's attachments directory and return
/// the resulting [`Attachment`] metadata.
pub async fn save(conversation_id: &str, source: &Path) -> Result<Attachment> {
    let meta = tokio::fs::metadata(source)
        .await
        .with_context(|| format!("stat {}", source.display()))?;
    if !meta.is_file() {
        return Err(anyhow!("attachment is not a file: {}", source.display()));
    }
    if meta.len() > MAX_ATTACHMENT_BYTES {
        return Err(anyhow!(
            "attachment is {} bytes; per-file cap is {} bytes",
            meta.len(),
            MAX_ATTACHMENT_BYTES
        ));
    }

    let dir = paths::attachments_dir()?.join(sanitize_id(conversation_id));
    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("mkdir {}", dir.display()))?;

    let name = source
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "attachment".into());
    let ext = source
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let id = Uuid::new_v4().to_string();
    let stored_name = if ext.is_empty() {
        id.clone()
    } else {
        format!("{id}.{ext}")
    };
    let dest = dir.join(&stored_name);

    tokio::fs::copy(source, &dest)
        .await
        .with_context(|| format!("copy {} -> {}", source.display(), dest.display()))?;

    let mime = mime_for(&name);
    let kind = classify(&mime).into();

    Ok(Attachment {
        kind,
        path: dest.to_string_lossy().to_string(),
        mime,
        bytes: meta.len(),
        name,
    })
}

/// Cheap path-only allow-list: strip anything that could escape the
/// attachments dir. UUID-shaped conversation ids pass through untouched;
/// anything wonky is reduced to a hex hash so we never write outside the
/// expected tree.
fn sanitize_id(id: &str) -> String {
    if id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !id.is_empty()
        && id.len() < 128
    {
        id.to_string()
    } else {
        use sha2::{Digest, Sha256};
        let h = Sha256::digest(id.as_bytes());
        hex(&h[..16])
    }
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Best-effort mime guess from filename. We only need to distinguish images
/// from everything else for the chat runner; the rest is informational.
pub fn mime_for(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "pdf" => "application/pdf",
        "json" => "application/json",
        "md" | "markdown" => "text/markdown",
        "txt" | "log" | "csv" | "tsv" => "text/plain",
        "html" | "htm" => "text/html",
        "xml" => "application/xml",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "py" | "rs" | "ts" | "tsx" | "js" | "jsx" | "go" | "rb" | "java" | "c" | "cpp" | "h"
        | "hpp" | "cs" | "sh" | "ps1" => "text/plain",
        _ => "application/octet-stream",
    }
    .to_string()
}

/// `image/*` → `"image"`, `audio/*` → `"audio"`, anything else → `"document"`.
pub fn classify(mime: &str) -> &'static str {
    if mime.starts_with("image/") {
        "image"
    } else if mime.starts_with("audio/") {
        "audio"
    } else {
        "document"
    }
}

/// Read an image attachment and produce the `data:<mime>;base64,<...>` URL
/// the OVMS chat endpoint expects in `image_url.url`. Bytes are read at
/// build-request time (rather than at upload time) so the conversation
/// payload stays small in SQLite.
pub async fn read_as_data_url(att: &Attachment) -> Result<String> {
    let bytes = tokio::fs::read(&att.path)
        .await
        .with_context(|| format!("read attachment {}", att.path))?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{};base64,{}", att.mime, encoded))
}

/// Read a document attachment as UTF-8 text. Returns `Ok(None)` when the
/// bytes don't decode (binary blob, image misclassified as document, ...);
/// the caller renders that as a `[binary file attached]` placeholder.
///
/// Oversized text is truncated and a `[truncated]` marker appended so the
/// model knows the snippet is incomplete.
pub async fn read_as_text(att: &Attachment) -> Result<Option<String>> {
    let bytes = tokio::fs::read(&att.path)
        .await
        .with_context(|| format!("read attachment {}", att.path))?;
    let Ok(text) = String::from_utf8(bytes) else {
        return Ok(None);
    };
    if text.len() > MAX_INLINE_TEXT_BYTES {
        // Walk back to a char boundary so we never split a multi-byte
        // codepoint when the cap lands mid-character.
        let mut cut = MAX_INLINE_TEXT_BYTES;
        while !text.is_char_boundary(cut) && cut > 0 {
            cut -= 1;
        }
        let mut clipped = text[..cut].to_string();
        clipped.push_str("\n\n[truncated]");
        return Ok(Some(clipped));
    }
    Ok(Some(text))
}

/// Best-effort wipe of the entire attachments tree for a conversation.
/// Used when the user deletes a conversation. Failure is non-fatal — the
/// orphaned files just sit there until the user clears them by hand.
pub fn purge_conversation(conversation_id: &str) -> Result<()> {
    let dir = paths::attachments_dir()?.join(sanitize_id(conversation_id));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).with_context(|| format!("remove {}", dir.display()))?;
    }
    Ok(())
}

#[allow(dead_code)]
pub fn dir_for(conversation_id: &str) -> Result<PathBuf> {
    Ok(paths::attachments_dir()?.join(sanitize_id(conversation_id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_for_extensions() {
        assert_eq!(mime_for("foo.PNG"), "image/png");
        assert_eq!(mime_for("doc.md"), "text/markdown");
        assert_eq!(mime_for("script.py"), "text/plain");
        assert_eq!(mime_for("data.bin"), "application/octet-stream");
        assert_eq!(mime_for("no_ext"), "application/octet-stream");
    }

    #[test]
    fn classify_buckets_by_prefix() {
        assert_eq!(classify("image/png"), "image");
        assert_eq!(classify("audio/wav"), "audio");
        assert_eq!(classify("application/pdf"), "document");
        assert_eq!(classify("text/markdown"), "document");
    }

    #[test]
    fn sanitize_keeps_uuid_shaped_ids() {
        let id = "8e9b0c54-0c64-4e6e-b3a4-2b1234567890";
        assert_eq!(sanitize_id(id), id);
    }

    #[test]
    fn sanitize_rejects_path_traversal() {
        let s = sanitize_id("../../escape");
        assert!(!s.contains(".."));
        assert!(!s.contains('/'));
    }
}
