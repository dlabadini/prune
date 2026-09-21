//! Local embeddings with fastembed, stored in sqlite-vec.

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub const EMBED_DIM: usize = 384;

fn embedder() -> Option<std::sync::MutexGuard<'static, TextEmbedding>> {
    static SLOT: OnceLock<Option<Mutex<TextEmbedding>>> = OnceLock::new();
    let slot = SLOT.get_or_init(|| {
        let options =
            TextInitOptions::new(EmbeddingModel::AllMiniLML6V2).with_show_download_progress(false);
        match TextEmbedding::try_new(options) {
            Ok(model) => Some(Mutex::new(model)),
            Err(error) => {
                tracing::warn!("local embedding model unavailable: {error}");
                None
            }
        }
    });
    slot.as_ref().and_then(|mutex| mutex.lock().ok())
}

/// Whether the MiniLM weights are already on disk. The first embed downloads them.
pub fn embedding_model_ready() -> bool {
    let cache = std::env::var("FASTEMBED_CACHE_DIR").unwrap_or_else(|_| ".fastembed_cache".into());
    onnx_present(&PathBuf::from(cache))
}

fn onnx_present(dir: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && onnx_present(&path) {
            return true;
        }
        if path.file_name().and_then(|name| name.to_str()) == Some("model.onnx") {
            return true;
        }
    }
    false
}

/// Embed a batch of texts. Returns nothing when the local model cannot load.
pub fn embed_texts(texts: &[String]) -> Option<Vec<Vec<f32>>> {
    if texts.is_empty() {
        return Some(Vec::new());
    }
    let mut model = embedder()?;
    let vectors = model.embed(texts, Some(32)).ok()?;
    if vectors.iter().any(|vector| vector.len() != EMBED_DIM) {
        tracing::warn!("embedding dimension did not match {EMBED_DIM}");
        return None;
    }
    Some(
        vectors
            .into_iter()
            .map(|vector| normalize(&vector))
            .collect(),
    )
}

pub fn normalize(vector: &[f32]) -> Vec<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm == 0.0 {
        return vector.to_vec();
    }
    vector.iter().map(|value| value / norm).collect()
}

pub fn to_blob(vector: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    blob
}

pub fn from_blob(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

/// Convert a unit-vector L2 distance from sqlite-vec into cosine similarity.
pub fn cosine_from_l2(distance: f32) -> f32 {
    (1.0 - (distance * distance) / 2.0).clamp(0.0, 1.0)
}

pub fn content_key(text: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_vectors_round_trip_and_score_as_identical() {
        let vector = normalize(&[1.0, 2.0, 3.0, 4.0]);
        let restored = from_blob(&to_blob(&vector));
        assert_eq!(restored.len(), vector.len());
        assert!((cosine_from_l2(0.0) - 1.0).abs() < f32::EPSILON);
    }
}
