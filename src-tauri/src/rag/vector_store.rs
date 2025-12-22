use super::{Document, SearchResult, FileInfo, FileInfoSummary};
use sled::Db;
use nalgebra::DVector;
use crate::paths;

// Database schema version for future migrations
const DB_SCHEMA_VERSION: &str = "v1.0.0";

pub struct VectorStore {
    db: Db,
}

impl VectorStore {
    pub fn new() -> Result<Self, String> {
        let data_dir = paths::get_vector_store_path().map_err(|e| e.to_string())?;
        
        // Try to open the database, with fallback for corruption or schema mismatch
        let db = match sled::open(&data_dir) {
            Ok(db) => {
                // Check if we can deserialize existing data
                if Self::validate_database_schema(&db) {
                    db
                } else {
                    // Schema mismatch - remove old database and create new one
                    drop(db); // Close the database first
                    
                    if data_dir.exists() {
                        if let Err(remove_err) = std::fs::remove_dir_all(&data_dir) {
                            return Err(format!("Failed to remove incompatible database: {}", remove_err));
                        }
                    }
                    
                    // Create parent directory again
                    if let Some(parent) = data_dir.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|e| format!("Failed to recreate data directory: {}", e))?;
                    }
                    
                    sled::open(&data_dir)
                        .map_err(|e| format!("Failed to create new database after schema migration: {}", e))?
                }
            }
            Err(_) => {
                // If the database is corrupted, try to remove it and create a new one
                if data_dir.exists() {
                    if let Err(remove_err) = std::fs::remove_dir_all(&data_dir) {
                        return Err(format!("Failed to remove corrupted database: {}", remove_err));
                    }
                }
                
                // Create parent directory again
                if let Some(parent) = data_dir.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("Failed to recreate data directory: {}", e))?;
                }
                
                // Try to open a fresh database
                sled::open(&data_dir)
                    .map_err(|e| format!("Failed to create new vector store after corruption recovery: {}", e))?
            }
        };
        
        // Store schema version for future migrations
        let _ = db.insert("__schema_version__", DB_SCHEMA_VERSION.as_bytes());
        
        Ok(Self { db })
    }
    
    /// Validate that existing database entries can be deserialized with current Document schema
    fn validate_database_schema(db: &Db) -> bool {
        // Check schema version first
        if let Ok(Some(version_bytes)) = db.get("__schema_version__") {
            if let Ok(version_str) = std::str::from_utf8(&version_bytes) {
                if version_str != DB_SCHEMA_VERSION {
                    return false;
                }
            }
        } else {
            // No version found - this means old database format
            return false;
        }
        
        let mut tested_count = 0;
        let max_test_entries = 5; // Only test a few entries for performance
        
        for item_result in db.iter() {
            if tested_count >= max_test_entries {
                break;
            }
            
            match item_result {
                Ok((key, value)) => {
                    // Skip metadata keys
                    if key.starts_with(b"__") {
                        continue;
                    }
                    
                    // Try to deserialize with current Document schema
                    match bincode::deserialize::<Document>(&value) {
                        Ok(doc) => {
                            // Additional validation - check if fields make sense
                            if doc.id.is_empty() || doc.content.is_empty() {
                                return false;
                            }
                            // Check if created_at is reasonable (not negative, not too far in future)
                            let now = chrono::Utc::now().timestamp_millis();
                            if doc.created_at < 0 || doc.created_at > now + 86400000 { // Allow 1 day in future
                                return false;
                            }
                        }
                        Err(_) => {
                            return false;
                        }
                    }
                    tested_count += 1;
                }
                Err(_) => {
                    return false;
                }
            }
        }
        
        true
    }
    
    pub fn store_document(&self, document: &Document) -> Result<(), String> {
        let key = document.id.as_bytes();
        let value = bincode::serialize(document)
            .map_err(|e| format!("Failed to serialize document: {}", e))?;
        
        self.db.insert(key, value)
            .map_err(|e| format!("Failed to store document: {}", e))?;
        
        Ok(())
    }
    
    
    pub fn search_similar(&self, query_embedding: &[f32], limit: usize) -> Result<Vec<SearchResult>, String> {
        let mut results = Vec::new();
        
        for item_result in self.db.iter() {
            match item_result {
                Ok((key, value)) => {
                    // Skip metadata keys
                    if key.starts_with(b"__") {
                        continue;
                    }
                    
                    match bincode::deserialize::<Document>(&value) {
                        Ok(document) => {
                            if let Some(embedding) = &document.embedding {
                                let similarity = cosine_similarity(query_embedding, embedding);
                                // Only add if similarity is valid (not NaN)
                                if similarity.is_finite() {
                                    results.push(SearchResult {
                                        document,
                                        score: similarity,
                                        rerank_score: None,
                                    });
                                }
                            }
                        }
                        Err(_) => {
                            // Skip corrupted documents
                            continue;
                        }
                    }
                }
                Err(_) => {
                    // Skip database iteration errors
                    continue;
                }
            }
        }
        
        // Sort by similarity score (highest first) with safe comparison
        results.sort_by(|a, b| {
            match (a.score.is_finite(), b.score.is_finite()) {
                (true, true) => b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal),
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                (false, false) => std::cmp::Ordering::Equal,
            }
        });
        results.truncate(limit);
        
        Ok(results)
    }
    
    pub fn delete_document(&self, id: &str) -> Result<bool, String> {
        let key = id.as_bytes();
        let result = self.db.remove(key)
            .map_err(|e| format!("Failed to delete document: {}", e))?;
        
        Ok(result.is_some())
    }
    
    pub fn list_all_documents(&self) -> Result<Vec<Document>, String> {
        let mut documents = Vec::new();
        let mut errors = Vec::new();
        
        // Use a safer iteration approach
        for item_result in self.db.iter() {
            match item_result {
                Ok((key, value)) => {
                    // Skip metadata keys
                    if key.starts_with(b"__") {
                        continue;
                    }
                    
                    match bincode::deserialize::<Document>(&value) {
                        Ok(document) => {
                            documents.push(document);
                        }
                        Err(e) => {
                            // Log deserialization error but don't fail the entire operation
                            errors.push(format!("Failed to deserialize document: {}", e));
                            continue;
                        }
                    }
                }
                Err(e) => {
                    // Log database error but don't fail the entire operation
                    errors.push(format!("Database iteration error: {}", e));
                    continue;
                }
            }
        }
        
        // Silently handle errors
        
        // Sort by creation time (newest first), with safe comparison
        documents.sort_by(|a, b| {
            b.created_at.cmp(&a.created_at)
        });
        
        Ok(documents)
    }
    
    pub fn count_documents(&self) -> Result<usize, String> {
        // Use a safer count method that actually iterates and counts valid documents
        let mut count = 0;
        
        for item_result in self.db.iter() {
            match item_result {
                Ok((key, value)) => {
                    // Skip metadata keys
                    if key.starts_with(b"__") {
                        continue;
                    }
                    
                    // Try to deserialize to make sure it's a valid document
                    if bincode::deserialize::<Document>(&value).is_ok() {
                        count += 1;
                    }
                }
                Err(_) => {
                    // Skip corrupted entries
                    continue;
                }
            }
        }
        
        Ok(count)
    }
    
    pub fn clear_all(&self) -> Result<(), String> {
        self.db.clear()
            .map_err(|e| format!("Failed to clear database: {}", e))?;
        Ok(())
    }
    
    pub fn list_files(&self) -> Result<Vec<FileInfo>, String> {
        let mut file_map: std::collections::HashMap<String, FileInfo> = std::collections::HashMap::new();
        
        for item_result in self.db.iter() {
            match item_result {
                Ok((key, value)) => {
                    // Skip metadata keys
                    if key.starts_with(b"__") {
                        continue;
                    }
                    
                    match bincode::deserialize::<Document>(&value) {
                        Ok(document) => {
                            // Safe key generation
                            let file_key = format!("{}:{}", 
                                document.file_path.trim(),
                                document.file_type.trim()
                            );
                            
                            match file_map.get_mut(&file_key) {
                                Some(file_info) => {
                                    file_info.chunk_count += 1;
                                    file_info.documents.push(document);
                                }
                                None => {
                                    // Safe file name extraction
                                    let safe_file_name = if document.file_path.is_empty() {
                                        document.title.clone()
                                    } else {
                                        match std::path::Path::new(&document.file_path).file_name() {
                                            Some(os_str) => {
                                                match os_str.to_str() {
                                                    Some(name) => name.to_string(),
                                                    None => document.title.clone(),
                                                }
                                            }
                                            None => document.title.clone(),
                                        }
                                    };
                                    
                                    let file_info = FileInfo {
                                        file_path: document.file_path.clone(),
                                        file_name: safe_file_name,
                                        file_type: document.file_type.clone(),
                                        chunk_count: 1,
                                        created_at: document.created_at,
                                        documents: vec![document],
                                    };
                                    file_map.insert(file_key, file_info);
                                }
                            }
                        }
                        Err(_) => {
                            // Skip corrupted documents
                            continue;
                        }
                    }
                }
                Err(_) => {
                    // Skip database iteration errors
                    continue;
                }
            }
        }
        
        let mut files: Vec<FileInfo> = file_map.into_values().collect();
        
        // Safe sorting with error handling
        files.sort_by(|a, b| {
            match (a.created_at, b.created_at) {
                (a_time, b_time) => b_time.cmp(&a_time),
            }
        });
        
        Ok(files)
    }
    
    pub fn delete_file(&self, file_path: &str) -> Result<usize, String> {
        let mut deleted_count = 0;
        let mut keys_to_delete = Vec::new();
        
        // Find all documents for this file
        for item_result in self.db.iter() {
            match item_result {
                Ok((key, value)) => {
                    // Skip metadata keys
                    if key.starts_with(b"__") {
                        continue;
                    }
                    
                    match bincode::deserialize::<Document>(&value) {
                        Ok(document) => {
                            if document.file_path == file_path {
                                keys_to_delete.push(key.to_vec());
                            }
                        }
                        Err(_) => {
                            // Skip corrupted documents
                            continue;
                        }
                    }
                }
                Err(_) => {
                    // Skip database iteration errors
                    continue;
                }
            }
        }
        
        // Delete all found keys
        for key in keys_to_delete {
            if let Ok(Some(_)) = self.db.remove(&key) {
                deleted_count += 1;
            }
        }
        
        Ok(deleted_count)
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    
    let vec_a = DVector::from_vec(a.to_vec());
    let vec_b = DVector::from_vec(b.to_vec());
    
    let dot_product = vec_a.dot(&vec_b);
    let norm_a = vec_a.norm();
    let norm_b = vec_b.norm();
    
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot_product / (norm_a * norm_b)
    }
}

#[tauri::command]
pub async fn store_documents(documents: Vec<Document>) -> Result<String, String> {
    if documents.is_empty() {
        return Ok("No documents to store".to_string());
    }

    let vector_store = VectorStore::new()?;
    
    for document in &documents {
        vector_store.store_document(document)?;
    }
    
    Ok(format!("Successfully stored {} documents", documents.len()))
}

#[tauri::command]
pub async fn search_documents(query_embedding: Vec<f32>, limit: Option<usize>) -> Result<Vec<SearchResult>, String> {
    let vector_store = VectorStore::new()?;
    let search_limit = limit.unwrap_or(10);
    
    vector_store.search_similar(&query_embedding, search_limit)
}

#[tauri::command]
pub async fn get_all_documents() -> Result<Vec<Document>, String> {
    let vector_store = VectorStore::new()?;
    vector_store.list_all_documents()
}

#[tauri::command]
pub async fn delete_document_by_id(id: String) -> Result<bool, String> {
    let vector_store = VectorStore::new()?;
    vector_store.delete_document(&id)
}

#[tauri::command]
pub async fn get_document_count() -> Result<usize, String> {
    let vector_store = VectorStore::new()?;
    vector_store.count_documents()
}

#[tauri::command]
pub async fn clear_all_documents() -> Result<String, String> {
    let vector_store = VectorStore::new()?;
    vector_store.clear_all()?;
    Ok("All documents cleared successfully".to_string())
}

#[tauri::command]
pub async fn get_all_files() -> Result<Vec<FileInfoSummary>, String> {
    let vector_store = VectorStore::new()?;
    let files = vector_store.list_files()?;
    
    // Convert FileInfo to FileInfoSummary to avoid serializing large document arrays
    let summaries: Vec<FileInfoSummary> = files.into_iter().map(|file| {
        FileInfoSummary {
            file_path: file.file_path,
            file_name: file.file_name,
            file_type: file.file_type,
            chunk_count: file.chunk_count,
            created_at: file.created_at,
        }
    }).collect();
    
    Ok(summaries)
}

#[tauri::command]
pub async fn get_file_chunks(#[allow(non_snake_case)] filePath: String) -> Result<Vec<Document>, String> {
    let vector_store = VectorStore::new()?;
    
    let mut chunks = Vec::new();
    
    for item_result in vector_store.db.iter() {
        match item_result {
            Ok((key, value)) => {
                // Skip metadata keys
                if key.starts_with(b"__") {
                    continue;
                }
                
                match bincode::deserialize::<Document>(&value) {
                    Ok(document) => {
                        if document.file_path == filePath {
                            chunks.push(document);
                        }
                    }
                    Err(_) => {
                        // Skip corrupted documents
                        continue;
                    }
                }
            }
            Err(_) => {
                // Skip database iteration errors
                continue;
            }
        }
    }
    
    // Sort by chunk index
    chunks.sort_by(|a, b| {
        match (a.chunk_index, b.chunk_index) {
            (Some(a_idx), Some(b_idx)) => a_idx.cmp(&b_idx),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.created_at.cmp(&b.created_at),
        }
    });
    
    Ok(chunks)
}

#[tauri::command]
pub async fn delete_file_by_path(#[allow(non_snake_case)] filePath: String) -> Result<usize, String> {
    let vector_store = VectorStore::new()?;
    vector_store.delete_file(&filePath)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        let similarity = cosine_similarity(&a, &b);
        assert!((similarity - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let similarity = cosine_similarity(&a, &b);
        assert!(similarity.abs() < 0.001);
    }
}