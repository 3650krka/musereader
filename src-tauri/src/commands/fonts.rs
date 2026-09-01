//! 阅读器自定义字体：
//! 用户导入的 TTF/OTF/WOFF/WOFF2 存入 `.runtime/fonts/`，
//! 前端经 asset 协议转 URL 注入 `@font-face`（无需本地 HTTP 服务）。

use super::{resolve_runtime_paths, AppError};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderFont {
    pub file_name: String,
    /// 展示名（去扩展名）。
    pub label: String,
    pub size_bytes: u64,
    /// 绝对路径（前端 convertFileSrc → asset URL）。
    pub path: String,
}

const FONT_EXTS: [&str; 4] = ["ttf", "otf", "woff", "woff2"];
const MAX_FONT_BYTES: u64 = 40 * 1024 * 1024;

fn reader_fonts_dir() -> PathBuf {
    resolve_runtime_paths()
        .artifact_root_dir
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("fonts")
}

fn list_fonts(dir: &Path) -> Vec<ReaderFont> {
    let mut fonts: Vec<ReaderFont> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().is_file())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| FONT_EXTS.contains(&e.to_ascii_lowercase().as_str()))
                .unwrap_or(false)
        })
        .filter_map(|entry| {
            let path = entry.path();
            let file_name = path.file_name()?.to_string_lossy().to_string();
            let label = path.file_stem()?.to_string_lossy().to_string();
            let size_bytes = entry.metadata().ok().map(|m| m.len()).unwrap_or(0);
            Some(ReaderFont {
                file_name,
                label,
                size_bytes,
                path: path.to_string_lossy().to_string(),
            })
        })
        .collect();
    fonts.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    fonts
}

/// 列出自定义字体（目录不存在时返回空表）。
#[tauri::command]
pub fn list_reader_fonts() -> Result<Vec<ReaderFont>, AppError> {
    Ok(list_fonts(&reader_fonts_dir()))
}

/// 导入字体：校验扩展/大小后复制进字体目录，并授权 asset 协议读取。
#[tauri::command]
pub async fn import_reader_font(
    source_path: String,
    app: AppHandle,
) -> Result<Vec<ReaderFont>, AppError> {
    let dir = reader_fonts_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::internal(format!("创建字体目录失败: {e}")))?;
    let src = PathBuf::from(&source_path);
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !FONT_EXTS.contains(&ext.as_str()) {
        return Err(AppError::invalid_input(
            "不支持的字体格式（仅 TTF/OTF/WOFF/WOFF2）",
        ));
    }
    let meta = std::fs::metadata(&src)
        .map_err(|e| AppError::invalid_input(format!("无法访问字体文件: {e}")))?;
    if !meta.is_file() {
        return Err(AppError::invalid_input("请选择字体文件"));
    }
    if meta.len() > MAX_FONT_BYTES {
        return Err(AppError::invalid_input("字体文件过大（>40MB）"));
    }
    let file_name = src
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| AppError::invalid_input("字体文件名无效"))?
        .to_string();
    let dest = dir.join(&file_name);
    std::fs::copy(&src, &dest)
        .map_err(|e| AppError::internal(format!("复制字体失败: {e}")))?;
    app.asset_protocol_scope()
        .allow_directory(&dir, true)
        .map_err(|e| AppError::internal(format!("授权字体目录失败: {e}")))?;
    Ok(list_fonts(&dir))
}

/// 删除字体（仅接受纯文件名，防路径逃逸；返回最新列表）。
#[tauri::command]
pub fn delete_reader_font(file_name: String) -> Result<Vec<ReaderFont>, AppError> {
    let dir = reader_fonts_dir();
    let candidate = PathBuf::from(&file_name);
    if candidate.components().count() == 1 {
        let target = dir.join(&file_name);
        if target.is_file() {
            std::fs::remove_file(&target)
                .map_err(|e| AppError::internal(format!("删除字体失败: {e}")))?;
        }
    }
    Ok(list_fonts(&dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn font_directory_lives_inside_runtime_root() {
        let dir = reader_fonts_dir();
        assert!(dir.ends_with("fonts"));
        assert!(dir
            .parent()
            .map(|p| p.ends_with(".runtime"))
            .unwrap_or(false));
    }
}
