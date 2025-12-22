use super::Document;
use pdf_extract::extract_text;
use calamine::{Reader, Xlsx, open_workbook};
use std::path::Path;
use std::fs;
use crate::constants;

#[tauri::command]
pub async fn process_document(file_path: String) -> Result<Vec<Document>, String> {
    log_operation_start!("Process document");
    
    let path = Path::new(&file_path);
    let extension = path.extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();

    tracing::debug!(file = %file_path, extension = %extension, "Processing document");

    let result = match extension.as_str() {
        "pdf" => process_pdf(&file_path).await,
        "docx" => process_docx(&file_path).await,
        "xlsx" | "xls" => process_excel(&file_path).await,
        _ => {
            log_operation_error!("Process document", "Unsupported file type", extension = %extension);
            Err("Unsupported file type".to_string())
        }
    }?;
    
    log_operation_success!("Process document");
    tracing::debug!(file = %file_path, chunks = result.len(), "Document processed into chunks");
    
    Ok(result)
}

#[tauri::command]
pub async fn save_temp_file(file_name: String, file_data: Vec<u8>) -> Result<String, String> {
    let temp_dir = std::env::temp_dir();
    let file_path = temp_dir.join(&file_name);
    
    fs::write(&file_path, file_data)
        .map_err(|e| format!("Failed to save temp file: {}", e))?;
    
    Ok(file_path.to_string_lossy().to_string())
}

async fn process_pdf(file_path: &str) -> Result<Vec<Document>, String> {
    let text = extract_text(file_path)
        .map_err(|e| {
            log_operation_error!("PDF extraction", &e, file = %file_path);
            format!("Failed to extract PDF text: {}", e)
        })?;
    
    tracing::debug!(file = %file_path, text_length = text.len(), "Extracted PDF text");
    
    let chunks = chunk_text(&text, constants::DEFAULT_CHUNK_SIZE, constants::DEFAULT_CHUNK_OVERLAP);
    
    let mut documents = Vec::new();
    let file_name = Path::new(file_path)
        .file_stem()
        .unwrap_or_default()
        .to_str()
        .unwrap_or("Unknown")
        .to_string();
    
    for (i, chunk) in chunks.iter().enumerate() {
        if chunk.trim().is_empty() {
            continue; // Skip empty chunks
        }
        
        documents.push(Document::new(
            format!("{} - Part {}", file_name, i + 1),
            chunk.clone(),
            "pdf".to_string(),
            file_path.to_string(),
            Some(i),
        ));
    }
    
    Ok(documents)
}

async fn process_docx(file_path: &str) -> Result<Vec<Document>, String> {
    // For now, we'll use a simple text extraction approach
    // You may want to use a more sophisticated DOCX parser
    let _file = fs::File::open(file_path)
        .map_err(|e| format!("Failed to open DOCX: {}", e))?;
    
    // Simple DOCX processing - you might want to use docx-rs properly
    let text = format!("DOCX content from: {}", file_path);
    
    let chunks = chunk_text(&text, constants::DEFAULT_CHUNK_SIZE, constants::DEFAULT_CHUNK_OVERLAP);
    
    let mut documents = Vec::new();
    let file_name = Path::new(file_path)
        .file_stem()
        .unwrap_or_default()
        .to_str()
        .unwrap_or("Unknown")
        .to_string();
    
    for (i, chunk) in chunks.iter().enumerate() {
        if chunk.trim().is_empty() {
            continue;
        }
        
        documents.push(Document::new(
            format!("{} - Part {}", file_name, i + 1),
            chunk.clone(),
            "docx".to_string(),
            file_path.to_string(),
            Some(i),
        ));
    }
    
    Ok(documents)
}

async fn process_excel(file_path: &str) -> Result<Vec<Document>, String> {
    let mut workbook: Xlsx<_> = open_workbook(file_path)
        .map_err(|e| format!("Failed to open Excel: {}", e))?;
    
    let mut documents = Vec::new();
    let file_name = Path::new(file_path)
        .file_stem()
        .unwrap_or_default()
        .to_str()
        .unwrap_or("Unknown")
        .to_string();
    
    for sheet_name in workbook.sheet_names().to_vec() {
        if let Ok(range) = workbook.worksheet_range(&sheet_name) {
            let mut text = String::new();
            text.push_str(&format!("Sheet: {}\n", sheet_name));
            
            for row in range.rows() {
                for cell in row {
                    text.push_str(&format!("{}\t", cell));
                }
                text.push('\n');
            }
            
            let chunks = chunk_text(&text, constants::DEFAULT_CHUNK_SIZE, constants::DEFAULT_CHUNK_OVERLAP);
            
            for (i, chunk) in chunks.iter().enumerate() {
                if chunk.trim().is_empty() {
                    continue;
                }
                
                documents.push(Document::new(
                    format!("{} - {} - Part {}", file_name, sheet_name, i + 1),
                    chunk.clone(),
                    "xlsx".to_string(),
                    file_path.to_string(),
                    Some(i),
                ));
            }
        }
    }
    
    Ok(documents)
}

fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    
    if chars.is_empty() {
        return chunks;
    }
    
    let mut start = 0;
    while start < chars.len() {
        let end = std::cmp::min(start + chunk_size, chars.len());
        let chunk: String = chars[start..end].iter().collect();
        
        if !chunk.trim().is_empty() {
            chunks.push(chunk);
        }
        
        if end == chars.len() {
            break;
        }
        start += chunk_size - overlap;
    }
    
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_text() {
        let text = "This is a test text that should be chunked properly.";
        let chunks = chunk_text(text, 20, 5);
        assert!(!chunks.is_empty());
        assert!(chunks[0].len() <= 20);
    }
}