use super::{resolve_runtime_paths, tasks::ensure_epub_path, AppError, AppState};
use super::{lock_mutex, persist_json};
use super::{BookProfile, ReaderState};
use super::tasks::{TaskArtifactPaths, TaskPhase, TaskStatus, TranslationTask};
use crate::document::pack::{read_opf_metadata, EpubBookMetadata};
use crate::document::{
    extract_epub_document, search_epub_reader, EpubReaderPackage, EpubSearchResult,
};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

/// 任务记录里的文件路径统一存正斜杠：Windows 反斜杠经 JSON 双重转义后，
/// 前端 `<img src>` 收到的是被破坏的相对路径（封面“加载不出来”根因之一）。
fn path_for_record(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpubProbeResult {
    pub markdown_chars: usize,
    pub paragraph_count: usize,
    pub has_cover: bool,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BilingualSegment {
    pub source: String,
    pub target: String,
    pub anchor_id: Option<String>,
}

#[tauri::command]
pub async fn import_pdf(filename: String, data: Vec<u8>) -> Result<String, AppError> {
    validate_import_payload(&filename, &data)?;
    let import_dir = resolve_runtime_paths().import_root_dir;
    tokio::fs::create_dir_all(&import_dir)
        .await
        .map_err(|error| AppError::internal(format!("创建导入目录失败: {error}")))?;

    let file_name = format!("{}_{}", Uuid::new_v4(), sanitize_filename(&filename));
    let output_path = import_dir.join(file_name);
    tokio::fs::write(&output_path, data)
        .await
        .map_err(|error| AppError::internal(format!("保存导入文件失败: {error}")))?;

    Ok(output_path.to_string_lossy().to_string())
}

fn validate_import_payload(filename: &str, data: &[u8]) -> Result<(), AppError> {
    if filename.trim().is_empty() {
        return Err(AppError::invalid_input("文件名不能为空"));
    }
    if data.is_empty() {
        return Err(AppError::invalid_input("导入内容为空"));
    }
    Ok(())
}
fn sanitize_filename(filename: &str) -> String {
    filename
        .chars()
        .map(|char| match char {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect()
}

#[tauri::command]
pub async fn read_translated_file(path: String) -> Result<String, AppError> {
    tokio::fs::read_to_string(&path)
        .await
        .map_err(|error| AppError::internal(format!("读取文件失败: {error}")))
}

/// 读取 EPUB 内置书名与作者，供书库展示真实元数据。
#[tauri::command]
pub async fn read_epub_metadata(path: String) -> Result<EpubBookMetadata, AppError> {
    let document_path = ensure_epub_path(&path)?;
    read_opf_metadata(&document_path)
}

#[tauri::command]
pub async fn prepare_epub_reader(
    task_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<EpubReaderPackage, AppError> {
    let epub_path = resolve_reader_epub_path(&state, &task_id)?;
    let cache_root = app
        .path()
        .app_cache_dir()
        .map_err(|error| AppError::internal(format!("定位应用缓存目录失败: {error}")))?
        .join("reader")
        .join("epub");
    // 目录覆盖层叠加（toc_overrides.json）：目录管理里的改名/合并对阅读器目录生效。
    let overrides = reader_toc_overrides(&task_id);
    let package = tokio::task::spawn_blocking(move || {
        crate::document::prepare_epub_reader_package_with_overrides(&epub_path, &cache_root, &overrides)
    })
    .await
    .map_err(|error| AppError::internal(format!("准备 EPUB 阅读包任务失败: {error}")))??;

    app.asset_protocol_scope()
        .allow_directory(Path::new(&package.root_path), true)
        .map_err(|error| AppError::internal(format!("授权 EPUB 阅读资源失败: {error}")))?;
    Ok(package)
}

/// 从 toc_overrides.json 提取 `(href, 修订标题)` 列表（叠加 renames；removed 条目
/// 不导出——阅读目录按 spine 全量展示，被合并条目其正文仍在原文件可读）。
fn reader_toc_overrides(task_id: &str) -> Vec<(String, String)> {
    let artifact_dir = resolve_runtime_paths().artifact_root_dir.join(task_id);
    let artifact = super::load_single_json::<crate::document::TocArtifact>(
        &artifact_dir.join("translated_toc.json"),
    )
    .or_else(|| {
        super::load_single_json::<crate::document::TocArtifact>(
            &artifact_dir.join("source_toc.json"),
        )
    });
    let Some(artifact) = artifact else { return Vec::new() };
    let Ok(overrides) = super::toc_edit::load_overrides(task_id) else {
        return Vec::new();
    };
    let mut applied = artifact;
    super::toc_edit::apply_toc_overrides(&mut applied, &overrides);
    applied
        .entries
        .iter()
        .filter_map(|entry| {
            let href = entry.href.as_deref()?.trim().to_string();
            let title = entry
                .translated_title
                .as_deref()
                .filter(|value| !value.trim().is_empty())?;
            Some((href, title.to_string()))
        })
        .collect()
}

#[tauri::command]
pub async fn search_epub_reader_content(
    task_id: String,
    query: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<EpubSearchResult>, AppError> {
    let epub_path = resolve_reader_epub_path(&state, &task_id)?;
    tokio::task::spawn_blocking(move || search_epub_reader(&epub_path, &query, limit.unwrap_or(40)))
        .await
        .map_err(|error| AppError::internal(format!("搜索 EPUB 阅读包任务失败: {error}")))?
}

/// 从 translated_blocks.json 读取真实双语段对（source → target 对齐）。
/// 供前端阅读器渲染真实双语对照，替换截取原文前 72 字符的伪译文。
#[tauri::command]
pub async fn read_bilingual_pairs(task_id: String) -> Result<Vec<BilingualSegment>, AppError> {
    let artifact_dir = resolve_runtime_paths().artifact_root_dir.join(&task_id);
    let blocks_path = artifact_dir.join("translated_blocks.json");
    let content = tokio::fs::read_to_string(&blocks_path)
        .await
        .map_err(|error| {
            AppError::internal(format!(
                "读取 translated_blocks.json 失败 ({}): {error}",
                blocks_path.display()
            ))
        })?;
    let blocks: Vec<crate::document::TranslatedBlock> =
        serde_json::from_str(&content).map_err(|error| {
            AppError::internal(format!("解析 translated_blocks.json 失败: {error}"))
        })?;
    let segments = blocks
        .into_iter()
        .filter(|b| b.translate && !b.source_text.trim().is_empty())
        .map(|b| BilingualSegment {
            source: b.source_text,
            target: b.translated_text,
            anchor_id: b.marker_id,
        })
        .collect();
    Ok(segments)
}

#[tauri::command]
pub async fn probe_epub(path: String) -> Result<EpubProbeResult, AppError> {
    let document_path = ensure_epub_path(&path)?;
    let document = extract_epub_document(&document_path)?;

    Ok(EpubProbeResult {
        markdown_chars: document.markdown.chars().count(),
        paragraph_count: count_markdown_paragraphs(&document.markdown),
        has_cover: document.cover_data_url.is_some(),
        preview: document.markdown.chars().take(320).collect(),
    })
}

fn count_markdown_paragraphs(markdown: &str) -> usize {
    markdown
        .split("\n\n")
        .filter(|paragraph| !paragraph.trim().is_empty())
        .count()
}

pub(crate) fn resolve_reader_epub_path(state: &AppState, task_id: &str) -> Result<PathBuf, AppError> {
    let task = {
        let tasks = super::lock_mutex(&state.tasks, "翻译任务")?;
        tasks
            .get(task_id)
            .cloned()
            .ok_or_else(|| AppError::not_found("未找到对应书籍任务"))?
    };
    let source_epub = is_epub_path(&task.pdf_path).then_some(task.pdf_path);
    let selected = task
        .artifact_paths
        .bilingual_epub_path
        .or(task.artifact_paths.translated_epub_path)
        .or(source_epub)
        .ok_or_else(|| AppError::not_found("该书籍没有可阅读的 EPUB 成书"))?;
    ensure_epub_path(&selected)
}

/// 为前端读取封面字节：asset 协议 scope 保持默认拒绝（不放开任意磁盘读取），
// 封面这种小图走白名单命令：canonicalize 后必须落在本机工件根目录内。
#[tauri::command]
pub async fn read_cover_data_url(path: String) -> Result<String, AppError> {
    let root = resolve_runtime_paths().artifact_root_dir
        .canonicalize()
        .map_err(|error| AppError::internal(format!("工件根目录不可用: {error}")))?;
    let target = PathBuf::from(path.trim())
        .canonicalize()
        .map_err(|_| AppError::not_found("封面文件不存在"))?;
    if !target.starts_with(&root) || !target.is_file() {
        return Err(AppError::invalid_input("仅允许读取工件目录内的封面文件"));
    }
    let mime = match target
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return Err(AppError::invalid_input("不支持的封面格式")),
    };
    let bytes = std::fs::read(&target)
        .map_err(|error| AppError::internal(format!("读取封面失败: {error}")))?;
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    Ok(format!("data:{mime};base64,{encoded}"))
}

fn is_epub_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("epub"))
}

/// 外部成书嗅探结果：本应用打包标记（双语/单语译文）或普通书。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EpubTranslationKind {
    /// 含双语对照 span（mt-zh + mt-en）。
    Bilingual,
    /// 仅中文译文 span（mt-zh）。
    Translated,
    /// 无本应用译文痕迹：按普通书收录。
    Plain,
}

/// 扫描 EPUB 正文判断译文来源：命中双语标记可提前终止，防大书全量解包。
fn detect_epub_translation_kind(path: &Path) -> EpubTranslationKind {
    let Ok(file) = std::fs::File::open(path) else {
        return EpubTranslationKind::Plain;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return EpubTranslationKind::Plain;
    };
    let mut has_zh = false;
    let mut has_en = false;
    for index in 0..archive.len() {
        let Ok(mut entry) = archive.by_index(index) else {
            continue;
        };
        let name = entry.name().to_ascii_lowercase();
        if !(name.ends_with(".xhtml") || name.ends_with(".html") || name.ends_with(".htm")) {
            continue;
        }
        let mut text = String::new();
        if std::io::Read::read_to_string(&mut entry, &mut text).is_err() {
            continue;
        }
        has_zh |= text.contains("mt-zh");
        has_en |= text.contains("mt-en");
        if has_zh && has_en {
            break;
        }
    }
    if has_zh && has_en {
        EpubTranslationKind::Bilingual
    } else if has_zh {
        EpubTranslationKind::Translated
    } else {
        EpubTranslationKind::Plain
    }
}

/// 迁移包：单本书的完整可携式归档 = 任务记录 + 阅读状态 + 档案 + 工件文件（artifacts/ 相对布局）。
const BUNDLE_MANIFEST: &str = "bundle.json";
const BUNDLE_DIR_PREFIX: &str = "artifacts/";
const BUNDLE_VERSION: u32 = 1;

/// 导出时按规范名兼带的工件文件（存在即入包）；任务记录里指向工件目录内的其它文件也会入包。
const BUNDLE_CANONICAL_FILES: &[&str] = &[
    "manifest.json",
    "translated.md",
    "translated.html",
    "source.md",
    "source_toc.json",
    "translated_toc.json",
    "translated_blocks.json",
    "glossary.json",
    "validation_report.json",
    "metrics.json",
    "packed/book_zh.epub",
    "packed/book_bilingual.epub",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BookBundle {
    version: u32,
    task: TranslationTask,
    #[serde(default)]
    reader_state: Option<ReaderState>,
    #[serde(default)]
    profile: Option<BookProfile>,
}

fn is_zip_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
}

/// zip 条目名安全化：只接受 artifacts/ 下的相对路径，拒绝穿越/绝对/盘符。
fn bundle_entry_rel(name: &str) -> Option<String> {
    let rel = name.strip_prefix(BUNDLE_DIR_PREFIX)?;
    if rel.is_empty()
        || rel.starts_with('/')
        || rel.contains("..")
        || rel.contains(':')
        || rel.contains('\\')
    {
        return None;
    }
    Some(rel.to_string())
}

/// 把序列化任务内所有引用旧工件目录前缀的字符串改写为新目录（跨机重根）。
fn reroot_value_strings(value: &mut serde_json::Value, old_prefix: &str, new_prefix: &str) {
    match value {
        serde_json::Value::String(text) => {
            if old_prefix.len() >= 4 && text.starts_with(old_prefix) {
                *text = format!("{new_prefix}{}", &text[old_prefix.len()..]);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                reroot_value_strings(item, old_prefix, new_prefix);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, item) in map {
                reroot_value_strings(item, old_prefix, new_prefix);
            }
        }
        _ => {}
    }
}

/// 导出单书迁移包 zip：任务/阅读状态/档案 + 工件目录必要文件（含任务记录引用的目录内文件）。
#[tauri::command]
pub async fn export_book_bundle(
    state: State<'_, AppState>,
    task_id: String,
    output_path: String,
) -> Result<String, AppError> {
    let output = PathBuf::from(output_path.trim());
    if output.as_os_str().is_empty() {
        return Err(AppError::invalid_input("导出路径为空"));
    }
    let (task, reader_state, profile) = {
        let tasks = lock_mutex(&state.tasks, "翻译任务")?;
        let task = tasks
            .get(&task_id)
            .cloned()
            .ok_or_else(|| AppError::not_found("未找到对应任务"))?;
        let reader_states = lock_mutex(&state.reader_states, "阅读状态")?;
        let profiles = lock_mutex(&state.book_profiles, "书籍档案")?;
        (
            task,
            reader_states.get(&task_id).cloned(),
            profiles.get(&task_id).cloned(),
        )
    };
    let Some(artifact_dir_text) = task.artifact_paths.artifact_dir.clone() else {
        return Err(AppError::invalid_input("该任务未记录工件目录，无法导出迁移包"));
    };
    let artifact_dir = PathBuf::from(&artifact_dir_text);
    if !artifact_dir.is_dir() {
        return Err(AppError::not_found(format!(
            "工件目录不存在: {artifact_dir_text}"
        )));
    }

    // 候选集：规范文件 + 任务记录绝对路径字段中落在工件目录内的文件（跨重打包目录兼容 packedN）。
    let mut candidates: Vec<String> = BUNDLE_CANONICAL_FILES.iter().map(|name| name.to_string()).collect();
    for record_path in [
        task.output_path.clone(),
        task.html_output_path.clone(),
        task.cover_path.clone(),
        task.artifact_paths.translated_epub_path.clone(),
        task.artifact_paths.bilingual_epub_path.clone(),
        task.artifact_paths.translated_toc_path.clone(),
        task.artifact_paths.source_toc_path.clone(),
        task.artifact_paths.glossary_path.clone(),
        task.artifact_paths.validation_report_path.clone(),
        task.artifact_paths.metrics_path.clone(),
        task.artifact_paths.manifest_path.clone(),
        task.artifact_paths.event_log_path.clone(),
        task.artifact_paths.output_markdown_path.clone(),
        task.artifact_paths.output_html_path.clone(),
    ]
    .into_iter()
    .flatten()
    {
        let Ok(relative) = Path::new(&record_path).strip_prefix(&artifact_dir) else {
            continue;
        };
        let rel = relative.to_string_lossy().replace('\\', "/");
        if !rel.is_empty() && !candidates.contains(&rel) {
            candidates.push(rel);
        }
    }
    // 译文的插图目录（translated.html 相对引用）整体随包。
    if artifact_dir.join("imgs").is_dir() && !candidates.iter().any(|rel| rel.starts_with("imgs/")) {
        candidates.push("imgs/**".to_string());
    }
    // 封面按磁盘实存收集（cover.<ext> 规范名）：导入时抽取落盘的封面在此进包。
    for name in ["cover.jpg", "cover.jpeg", "cover.png", "cover.webp"] {
        if artifact_dir.join(name).is_file() && !candidates.iter().any(|rel| rel == name) {
            candidates.push(name.to_string());
        }
    }

    let bundle = BookBundle {
        version: BUNDLE_VERSION,
        task,
        reader_state,
        profile,
    };
    tokio::task::spawn_blocking(move || write_bundle_zip(output, artifact_dir, candidates, &bundle))
        .await
        .map_err(|error| AppError::internal(format!("导出任务失败: {error}")))?
}

fn write_bundle_zip(
    output: PathBuf,
    artifact_dir: PathBuf,
    candidates: Vec<String>,
    bundle: &BookBundle,
) -> Result<String, AppError> {
    let file = std::fs::File::create(&output)
        .map_err(|error| AppError::internal(format!("创建迁移包失败: {error}")))?;
    let mut writer = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    writer
        .start_file(BUNDLE_MANIFEST, options)
        .map_err(|error| AppError::internal(format!("写入清单失败: {error}")))?;
    let manifest_bytes = serde_json::to_vec_pretty(bundle)
        .map_err(|error| AppError::internal(format!("序列化迁移清单失败: {error}")))?;
    writer
        .write_all(&manifest_bytes)
        .map_err(|error| AppError::internal(format!("写入清单内容失败: {error}")))?;

    let mut packed_count = 0usize;
    for rel in candidates {
        if let Some(stripped) = rel.strip_suffix("/**") {
            // 目录整体打包（imgs）。
            let dir = artifact_dir.join(stripped);
            collect_dir_files(&dir, stripped, &mut |entry_rel, entry_path| {
                add_bundle_file(&mut writer, options, entry_rel, entry_path).map(|()| packed_count += 1)
            })?;
            continue;
        }
        let source = artifact_dir.join(&rel);
        if !source.is_file() {
            continue;
        }
        add_bundle_file(&mut writer, options, &rel, &source)?;
        packed_count += 1;
    }
    writer
        .finish()
        .map_err(|error| AppError::internal(format!("收尾迁移包失败: {error}")))?;
    let _ = packed_count;
    Ok(output.to_string_lossy().to_string())
}

fn add_bundle_file(
    writer: &mut ZipWriter<std::fs::File>,
    options: SimpleFileOptions,
    rel: &str,
    source: &Path,
) -> Result<(), AppError> {
    let mut data = Vec::new();
    let file = std::fs::File::open(source)
        .map_err(|error| AppError::internal(format!("读取工件失败 {rel}: {error}")))?;
    let mut reader = std::io::BufReader::new(file);
    reader
        .read_to_end(&mut data)
        .map_err(|error| AppError::internal(format!("读取工件内容失败 {rel}: {error}")))?;
    writer
        .start_file(format!("{BUNDLE_DIR_PREFIX}{rel}"), options)
        .map_err(|error| AppError::internal(format!("写入工件条目失败 {rel}: {error}")))?;
    writer
        .write_all(&data)
        .map_err(|error| AppError::internal(format!("写入工件内容失败 {rel}: {error}")))?;
    Ok(())
}

/// 递归收集目录内文件（防符号链狂走：深度上限 6，条目上限 4000）。
fn collect_dir_files<F>(
    dir: &Path,
    prefix_rel: &str,
    visit: &mut F,
) -> Result<(), AppError>
where
    F: FnMut(&str, &Path) -> Result<(), AppError>,
{
    fn walk<F>(
        current: &Path,
        prefix_rel: &str,
        depth: u32,
        budget: &mut usize,
        visit: &mut F,
    ) -> Result<(), AppError>
    where
        F: FnMut(&str, &Path) -> Result<(), AppError>,
    {
        if depth > 6 {
            return Ok(());
        }
        let Ok(entries) = std::fs::read_dir(current) else {
            return Ok(());
        };
        for entry in entries.flatten() {
            if *budget == 0 {
                return Ok(());
            }
            *budget -= 1;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().replace('\\', "/");
            let rel = format!("{prefix_rel}/{name}");
            if path.is_dir() {
                walk(&path, prefix_rel, depth + 1, budget, visit)?;
            } else if path.is_file() {
                visit(&rel, &path)?;
            }
        }
        Ok(())
    }
    let mut budget = 4000usize;
    walk(dir, prefix_rel, 0, &mut budget, visit)
}

/// 数据迁移入口：接受三种形态——迁移包 zip / 工件目录 / 单本成书 EPUB。
/// zip 与目录恢复完整工件集（阅读/校对/目录/日志全功能）；裸 EPUB 登记为只读成书。
#[tauri::command]
pub async fn import_translated_book(
    state: State<'_, AppState>,
    path: String,
) -> Result<TranslationTask, AppError> {
    let source = PathBuf::from(path.trim());
    if source.is_dir() {
        return import_artifact_folder(&state, &source).await;
    }
    if is_zip_path(&source) {
        return import_book_bundle(&state, &source).await;
    }
    // 文件对话框选不了目录：用户进工件目录选中 manifest.json / translated.md 即视为导入整个目录。
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(file_name.as_str(), "manifest.json" | "translated.md") {
        if let Some(parent) = source.parent() {
            return import_artifact_folder(&state, parent).await;
        }
    }
    import_bare_epub_book(&state, &source).await
}

/// 工件目录内若无封面图，从成书 EPUB 抽取写盘（优先双语版）；返回记录用正斜杠路径。
fn ensure_artifact_cover(artifact_dir: &Path, epub_candidates: &[PathBuf]) -> Option<String> {
    for name in ["cover.jpg", "cover.jpeg", "cover.png", "cover.webp"] {
        let existing = artifact_dir.join(name);
        if existing.is_file() {
            return Some(path_for_record(&existing));
        }
    }
    for epub in epub_candidates {
        if !epub.is_file() {
            continue;
        }
        let Ok(document) = extract_epub_document(epub) else {
            continue;
        };
        let Some(data_url) = document.cover_data_url else {
            continue;
        };
        let Some(rest) = data_url.strip_prefix("data:image/") else {
            continue;
        };
        let Some((mime, payload)) = rest.split_once(";base64,") else {
            continue;
        };
        let ext = match mime {
            "jpeg" | "jpg" => "jpg",
            "png" => "png",
            "webp" => "webp",
            _ => continue,
        };
        let Ok(bytes) = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload) else {
            continue;
        };
        if bytes.len() < 1024 {
            continue; // 占位小图无意义。
        }
        let dest = artifact_dir.join(format!("cover.{ext}"));
        if std::fs::write(&dest, &bytes).is_ok() {
            return Some(path_for_record(&dest));
        }
    }
    None
}

fn cover_needs_refresh(path: Option<&str>) -> bool {
    match path {
        Some(value) => !Path::new(value).is_file(),
        None => true,
    }
}

/// 迁移包导入：解包工件到本机工件目录（同 task id 优先复用，阅读笔记自动接回；
/// 本地已有同 id 任务且包内无阅读状态时不覆盖本地）。
async fn import_book_bundle(state: &State<'_, AppState>, zip_path: &Path) -> Result<TranslationTask, AppError> {
    let zip_path = zip_path.to_path_buf();
    if !zip_path.is_file() {
        return Err(AppError::invalid_input("迁移包文件不存在"));
    }
    let artifact_root = state.artifact_root_dir.clone();
    let prepared = tokio::task::spawn_blocking(move || -> Result<(TranslationTask, Option<ReaderState>, Option<BookProfile>), AppError> {
        let file = std::fs::File::open(&zip_path)
            .map_err(|error| AppError::invalid_input(format!("打开迁移包失败: {error}")))?;
        let mut archive = ZipArchive::new(file)
            .map_err(|error| AppError::invalid_input(format!("迁移包不是合法 zip: {error}")))?;
        let manifest_text = {
            let mut handle = archive
                .by_name(BUNDLE_MANIFEST)
                .map_err(|_| AppError::invalid_input("迁移包缺 bundle.json（请确认由本应用导出的包）"))?;
            let mut text = String::new();
            handle
                .read_to_string(&mut text)
                .map_err(|error| AppError::invalid_input(format!("读取 bundle.json 失败: {error}")))?;
            text
        };
        let bundle: BookBundle = serde_json::from_str(&manifest_text)
            .map_err(|error| AppError::invalid_input(format!("bundle.json 解析失败: {error}")))?;
        if bundle.version != BUNDLE_VERSION {
            return Err(AppError::invalid_input(format!(
                "迁移包版本不兼容: {}", bundle.version
            )));
        }
        let old_prefix = bundle
            .task
            .artifact_paths
            .artifact_dir
            .clone()
            .unwrap_or_default();
        let task_id = bundle.task.id.clone();
        if task_id.trim().is_empty() || task_id.len() > 128 || task_id.contains(['/', '\\', ':']) {
            return Err(AppError::invalid_input("迁移包任务 id 非法"));
        }
        let target_dir = artifact_root.join(&task_id);
        std::fs::create_dir_all(&target_dir)
            .map_err(|error| AppError::internal(format!("创建工件目录失败: {error}")))?;
        let mut extracted = 0usize;
        for index in 0..archive.len() {
            let mut entry = archive
                .by_index(index)
                .map_err(|error| AppError::internal(format!("读迁移包条目失败: {error}")))?;
            let Some(rel) = bundle_entry_rel(entry.name()) else { continue };
            let dest = target_dir.join(&rel);
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let mut out = std::fs::File::create(&dest)
                .map_err(|error| AppError::internal(format!("写工件文件失败 {rel}: {error}")))?;
            let copied = std::io::copy(&mut entry, &mut out)
                .map_err(|error| AppError::internal(format!("解包失败 {rel}: {error}")))?;
            drop(out);
            if copied == 0 && !rel.ends_with(".md") {
                let _ = std::fs::remove_file(&dest);
                continue;
            }
            extracted += 1;
        }
        if extracted == 0 {
            return Err(AppError::invalid_input("迁移包内无工件文件"));
        }
        let target_text = target_dir.to_string_lossy().to_string();
        let mut value = serde_json::to_value(&bundle.task)
            .map_err(|error| AppError::internal(format!("任务序列化失败: {error}")))?;
        if !old_prefix.is_empty() {
            reroot_value_strings(&mut value, &old_prefix, &target_text);
        }
        if let Some(dir_field) = value
            .get_mut("artifactPaths")
            .and_then(|paths| paths.get_mut("artifactDir"))
        {
            *dir_field = serde_json::Value::String(target_text.clone());
        }
        let mut task: TranslationTask = serde_json::from_value(value)
            .map_err(|error| AppError::internal(format!("任务重建失败: {error}")))?;
        task.message = "已从迁移包恢复".to_string();
        task.updated_at = chrono::Utc::now().to_rfc3339();
        if cover_needs_refresh(task.cover_path.as_deref()) {
            task.cover_path = ensure_artifact_cover(
                &target_dir,
                &[
                    target_dir.join("packed").join("book_bilingual.epub"),
                    target_dir.join("packed").join("book_zh.epub"),
                ],
            );
        }
        Ok((task, bundle.reader_state, bundle.profile))
    })
    .await
    .map_err(|error| AppError::internal(format!("解包任务失败: {error}")))??;

    let (task, reader_state, profile) = prepared;
    register_restored_book(state, task, reader_state, profile)
}

/// 裸工件目录导入（无 bundle.json 的早期形态/手工拷目录）：按规范名扫描重建任务记录。
async fn import_artifact_folder(state: &State<'_, AppState>, dir: &Path) -> Result<TranslationTask, AppError> {
    let artifact_root = state.artifact_root_dir.clone();
    let dir = dir.to_path_buf();
    let prepared = tokio::task::spawn_blocking(move || -> Result<TranslationTask, AppError> {
        if !dir.join("translated.md").is_file() && !dir.join("packed").join("book_zh.epub").is_file() {
            return Err(AppError::invalid_input(
                "该目录不是译作工件目录（缺 translated.md / packed/book_zh.epub）",
            ));
        }
        let folder_manifest: Option<serde_json::Value> = std::fs::read_to_string(dir.join("manifest.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok());
        let mut task_id = folder_manifest
            .as_ref()
            .and_then(|value| value.get("taskId"))
            .and_then(serde_json::Value::as_str)
            .map(|value| value.to_string())
            .unwrap_or_else(|| dir.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default());
        if task_id.trim().is_empty() || task_id.len() > 128 || task_id.contains(['/', '\\', ':']) {
            task_id = Uuid::new_v4().simple().to_string();
        }
        let target_dir = if artifact_root.join(&task_id).exists() {
            task_id = Uuid::new_v4().simple().to_string();
            artifact_root.join(&task_id)
        } else {
            artifact_root.join(&task_id)
        };
        std::fs::create_dir_all(&target_dir)
            .map_err(|error| AppError::internal(format!("创建工件目录失败: {error}")))?;

        let mut candidates: Vec<String> = BUNDLE_CANONICAL_FILES.iter().map(|name| name.to_string()).collect();
        if dir.join("imgs").is_dir() {
            candidates.push("imgs/**".to_string());
        }
        let mut copied = 0usize;
        for rel in candidates {
            if let Some(stripped) = rel.strip_suffix("/**") {
                let source = dir.join(stripped);
                let mut entries: Vec<(String, PathBuf)> = Vec::new();
                collect_dir_files(&source, stripped, &mut |entry_rel, entry_path| {
                    entries.push((entry_rel.to_string(), entry_path.to_path_buf()));
                    Ok(())
                })?;
                for (entry_rel, entry_path) in entries {
                    let dest = target_dir.join(&entry_rel);
                    if let Some(parent) = dest.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if std::fs::copy(&entry_path, &dest).is_ok() {
                        copied += 1;
                    }
                }
                continue;
            }
            let source = dir.join(&rel);
            if !source.is_file() {
                continue;
            }
            let dest = target_dir.join(&rel);
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::copy(&source, &dest).is_ok() {
                copied += 1;
            }
        }
        if copied == 0 {
            return Err(AppError::invalid_input("目录内没有可导入的必要工件文件"));
        }

        let text_of = |key: &str| -> Option<String> {
            folder_manifest
                .as_ref()
                .and_then(|value| value.get(key))
                .and_then(serde_json::Value::as_str)
                .map(|value| value.to_string())
        };
        let chunks_of = |key: &str| -> usize {
            folder_manifest
                .as_ref()
                .and_then(|value| value.get(key))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default() as usize
        };
        let path_of = |name: &str| -> Option<String> {
            target_dir.join(name).is_file().then(|| target_dir.join(name).to_string_lossy().to_string())
        };

        // 书名优先级：成书 OPF 元数据 → translated.md front matter → 目录名。
        let opf_title = [
            target_dir.join("packed").join("book_bilingual.epub"),
            target_dir.join("packed").join("book_zh.epub"),
        ]
        .into_iter()
        .find(|path| path.is_file())
        .and_then(|path| read_opf_metadata(&path).ok())
        .and_then(|meta| meta.title)
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty());
        let title = opf_title.or_else(|| title_from_markdown(&target_dir.join("translated.md")));
        let cover_path = ensure_artifact_cover(
            &target_dir,
            &[
                target_dir.join("packed").join("book_bilingual.epub"),
                target_dir.join("packed").join("book_zh.epub"),
            ],
        );
        let filename = title.unwrap_or_else(|| {
            dir.file_name()
                .map(|name| format!("{}（外部导入）", name.to_string_lossy()))
                .unwrap_or_else(|| "外部导入译作".to_string())
        });
        let now = chrono::Utc::now().to_rfc3339();
        Ok(TranslationTask {
            id: task_id,
            filename,
            pdf_path: text_of("sourcePdfPath").unwrap_or_default(),
            status: TaskStatus::Completed,
            phase: TaskPhase::Completed,
            progress: 100,
            message: "已从工件目录导入".to_string(),
            article_type: text_of("articleType").unwrap_or_else(|| "general".to_string()),
            concurrent: false,
            max_workers: 1,
            created_at: text_of("createdAt").unwrap_or_else(|| now.clone()),
            updated_at: now,
            output_path: path_of("translated.md"),
            html_output_path: path_of("translated.html"),
            cover_path,
            artifact_paths: TaskArtifactPaths {
                artifact_dir: Some(target_dir.to_string_lossy().to_string()),
                source_markdown_path: path_of("source.md"),
                source_toc_path: path_of("source_toc.json"),
                translated_toc_path: path_of("translated_toc.json"),
                output_markdown_path: path_of("translated.md"),
                output_html_path: path_of("translated.html"),
                translated_epub_path: path_of("packed/book_zh.epub"),
                bilingual_epub_path: path_of("packed/book_bilingual.epub"),
                manifest_path: path_of("manifest.json"),
                event_log_path: path_of("events.ndjson"),
                validation_report_path: path_of("validation_report.json"),
                glossary_path: path_of("glossary.json"),
                metrics_path: path_of("metrics.json"),
                ..TaskArtifactPaths::default()
            },
            total_chunks: chunks_of("totalChunks"),
            translated_chunks: chunks_of("translatedChunks"),
            retry_count: 0,
            source_hash: text_of("sourceHash"),
            last_error: None,
        })
    })
    .await
    .map_err(|error| AppError::internal(format!("导入任务失败: {error}")))??;

    register_restored_book(state, prepared, None, None)
}

/// 从 translated.md 头部 front matter 提书名（前 64 行内找 title）。
fn title_from_markdown(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    let mut head = String::new();
    // 只读头部：最多 8KB 足够 front matter。
    let mut chunk = [0u8; 8192];
    let read = reader.read(&mut chunk).ok()?;
    head.push_str(&String::from_utf8_lossy(&chunk[..read]));
    for line in head.lines().take(64) {
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("title:").or_else(|| trimmed.strip_prefix("Title:")) {
            let value = value.trim().trim_matches('"').trim_matches('\'').to_string();
            if !value.is_empty() {
                return Some(value);
            }
        }
        if !head.starts_with("---") {
            break;
        }
    }
    None
}

/// 导入登记：写三表并持久化；失败回滚内存。
fn register_restored_book(
    state: &State<'_, AppState>,
    task: TranslationTask,
    reader_state: Option<ReaderState>,
    profile: Option<BookProfile>,
) -> Result<TranslationTask, AppError> {
    let task_id = task.id.clone();
    let previous_tasks = lock_mutex(&state.tasks, "翻译任务")?.clone();
    let previous_reader = lock_mutex(&state.reader_states, "阅读状态")?.clone();
    let previous_profiles = lock_mutex(&state.book_profiles, "书籍档案")?.clone();

    lock_mutex(&state.tasks, "翻译任务")?.insert(task_id.clone(), task.clone());
    if let Some(mut state_row) = reader_state {
        state_row.book_id = task_id.clone();
        lock_mutex(&state.reader_states, "阅读状态")?.insert(task_id.clone(), state_row);
    }
    if let Some(mut row_profile) = profile {
        row_profile.book_id = task_id.clone();
        lock_mutex(&state.book_profiles, "书籍档案")?.insert(task_id.clone(), row_profile);
    }

    let persist_result = persist_json(&state.task_store_path, &*lock_mutex(&state.tasks, "翻译任务")?)
        .and_then(|()| persist_json(&state.reader_state_store_path, &*lock_mutex(&state.reader_states, "阅读状态")?))
        .and_then(|()| persist_json(&state.book_profile_store_path, &*lock_mutex(&state.book_profiles, "书籍档案")?));
    if let Err(error) = persist_result {
        *lock_mutex(&state.tasks, "翻译任务")? = previous_tasks;
        *lock_mutex(&state.reader_states, "阅读状态")? = previous_reader;
        *lock_mutex(&state.book_profiles, "书籍档案")? = previous_profiles;
        return Err(error);
    }
    super::tasks::emit_task_progress(&state.task_event_sink, &task);
    Ok(task)
}

/// 单本成书 EPUB 导入（无工件集）：复制进导入目录，登记为只读成书；
/// 双语/单语/原语由正文 span 标记嗅探。
async fn import_bare_epub_book(
    state: &State<'_, AppState>,
    _source_hint: &Path,
) -> Result<TranslationTask, AppError> {
    let source = ensure_epub_path(&_source_hint.to_string_lossy())?;
    let kind = detect_epub_translation_kind(&source);
    let metadata = read_opf_metadata(&source).ok();
    let original_name = source
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "imported.epub".to_string());

    let import_dir = resolve_runtime_paths().import_root_dir;
    tokio::fs::create_dir_all(&import_dir)
        .await
        .map_err(|error| AppError::internal(format!("创建导入目录失败: {error}")))?;
    let task_id = Uuid::new_v4().simple().to_string();
    let dest = import_dir.join(format!("{task_id}_{}", sanitize_filename(&original_name)));
    tokio::fs::copy(&source, &dest)
        .await
        .map_err(|error| AppError::internal(format!("复制成书失败: {error}")))?;

    let dest_text = dest.to_string_lossy().to_string();
    // 裸成书也建工件目录并抽封面：书架卡面不再是纯色占位；删除任务时随目录回收。
    let artifact_dir = resolve_runtime_paths().artifact_root_dir.join(&task_id);
    let _ = std::fs::create_dir_all(&artifact_dir);
    let cover_path = ensure_artifact_cover(&artifact_dir, std::slice::from_ref(&dest));
    let mut artifact_paths = TaskArtifactPaths {
        artifact_dir: Some(artifact_dir.to_string_lossy().to_string()),
        ..TaskArtifactPaths::default()
    };
    match kind {
        EpubTranslationKind::Bilingual => {
            artifact_paths.bilingual_epub_path = Some(dest_text.clone());
            artifact_paths.translated_epub_path = Some(dest_text);
        }
        EpubTranslationKind::Translated => artifact_paths.translated_epub_path = Some(dest_text),
        EpubTranslationKind::Plain => {}
    }
    let now = chrono::Utc::now().to_rfc3339();
    // 书名优先用 EPUB 元数据（packed14 产物文件名统一为 book_bilingual.epub，直接用作标题不可读）。
    let filename = metadata
        .as_ref()
        .and_then(|value: &EpubBookMetadata| value.title.clone())
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty())
        .unwrap_or(original_name);
    let task = TranslationTask {
        id: task_id,
        filename,
        pdf_path: dest.to_string_lossy().to_string(),
        status: TaskStatus::Completed,
        phase: TaskPhase::Completed,
        progress: 100,
        message: match kind {
            EpubTranslationKind::Bilingual => "已导入（双语成书）",
            EpubTranslationKind::Translated => "已导入（译文成书）",
            EpubTranslationKind::Plain => "已导入（未检测到译文标记）",
        }
        .to_string(),
        article_type: "general".to_string(),
        concurrent: false,
        max_workers: 1,
        created_at: now.clone(),
        updated_at: now,
        output_path: None,
        html_output_path: None,
        cover_path,
        artifact_paths,
        total_chunks: 0,
        translated_chunks: 0,
        retry_count: 0,
        source_hash: None,
        last_error: None,
    };

    let snapshot = {
        let mut tasks = lock_mutex(&state.tasks, "翻译任务")?;
        tasks.insert(task.id.clone(), task.clone());
        tasks.clone()
    };
    persist_json(&state.task_store_path, &snapshot)?;
    super::tasks::emit_task_progress(&state.task_event_sink, &task);
    Ok(task)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    fn write_marker_epub(label: &str, body: &str) -> PathBuf {
        let mut path = std::env::temp_dir().join(format!(
            "musereader-import-sniff-{}-{}",
            label,
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        path.set_extension("epub");
        let file = std::fs::File::create(&path).expect("temp epub should be creatable");
        let mut writer = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        writer.start_file("OEBPS/chapter.xhtml", options).expect("entry");
        writer.write_all(body.as_bytes()).expect("write");
        writer.finish().expect("zip finish");
        path
    }

    #[test]
    fn sniff_detects_bilingual_single_language_and_plain_books() {
        let bilingual = write_marker_epub(
            "bilingual",
            "<p><span class=\"mt-zh\">中文</span><br/><span class=\"mt-en\">text</span></p>",
        );
        let translated = write_marker_epub("zh", "<p><span class=\"mt-zh\">只有中文</span></p>");
        let plain = write_marker_epub("plain", "<p>English only body.</p>");
        assert_eq!(detect_epub_translation_kind(&bilingual), EpubTranslationKind::Bilingual);
        assert_eq!(detect_epub_translation_kind(&translated), EpubTranslationKind::Translated);
        assert_eq!(detect_epub_translation_kind(&plain), EpubTranslationKind::Plain);
        // 非 zip 文件防御：不报错，按普通书处理。
        let garbage = std::env::temp_dir().join(format!("musereader-import-garbage-{}", Uuid::new_v4()));
        std::fs::write(&garbage, b"not a zip").expect("garbage");
        assert_eq!(detect_epub_translation_kind(&garbage), EpubTranslationKind::Plain);
        for path in [bilingual, translated, plain, garbage] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn bundle_entry_rel_rejects_traversal_and_accepts_layout() {
        assert_eq!(
            bundle_entry_rel("artifacts/packed/book_zh.epub").as_deref(),
            Some("packed/book_zh.epub")
        );
        assert_eq!(bundle_entry_rel("bundle.json"), None);
        assert_eq!(bundle_entry_rel("artifacts/../escape.md"), None);
        assert_eq!(bundle_entry_rel("artifacts//"), None);
        assert_eq!(bundle_entry_rel("artifacts/C:/evil.md"), None);
    }

    #[test]
    fn reroot_value_strings_rewrites_only_old_prefix() {
        let mut value = serde_json::json!({
            "artifactPaths": { "artifactDir": "D:\\old\\artifacts\\t1", "outputMarkdownPath": "D:\\old\\artifacts\\t1\\translated.md" },
            "pdfPath": "D:\\elsewhere\\source.epub",
            "progress": 100
        });
        reroot_value_strings(&mut value, "D:\\old\\artifacts\\t1", "E:\\new\\t1");
        assert_eq!(
            value["artifactPaths"]["outputMarkdownPath"].as_str().unwrap(),
            "E:\\new\\t1\\translated.md"
        );
        assert_eq!(value["pdfPath"].as_str().unwrap(), "D:\\elsewhere\\source.epub");
    }

    #[test]
    fn bundle_zip_roundtrip_keeps_manifest_and_files() {
        let dir = std::env::temp_dir().join(format!(
            "musereader-bundle-{}",
            Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(dir.join("packed")).expect("artifact dir");
        std::fs::write(
            dir.join("translated.md"),
            "---\ntitle: 测试之书\n---\n\n正文\n",
        )
        .expect("md");
        std::fs::write(dir.join("packed").join("book_zh.epub"), b"epub-bytes").expect("epub");
        let zip_path = dir.join("out.zip");
        let bundle = BookBundle {
            version: BUNDLE_VERSION,
            task: TranslationTask {
                id: "t1".to_string(),
                filename: "x.epub".to_string(),
                pdf_path: String::new(),
                status: TaskStatus::Completed,
                phase: TaskPhase::Completed,
                progress: 100,
                message: String::new(),
                article_type: "general".to_string(),
                concurrent: false,
                max_workers: 1,
                created_at: "2026-01-01T00:00:00Z".to_string(),
                updated_at: "2026-01-01T00:00:00Z".to_string(),
                output_path: Some(dir.join("translated.md").to_string_lossy().to_string()),
                html_output_path: None,
                cover_path: None,
                artifact_paths: TaskArtifactPaths {
                    artifact_dir: Some(dir.to_string_lossy().to_string()),
                    ..TaskArtifactPaths::default()
                },
                total_chunks: 3,
                translated_chunks: 3,
                retry_count: 0,
                source_hash: None,
                last_error: None,
            },
            reader_state: None,
            profile: None,
        };
        let candidates: Vec<String> = BUNDLE_CANONICAL_FILES.iter().map(|v| v.to_string()).collect();
        let written = write_bundle_zip(zip_path.clone(), dir.clone(), candidates, &bundle)
            .expect("bundle zip should be written");
        let file = std::fs::File::open(&written).expect("open zip");
        let mut archive = ZipArchive::new(file).expect("zip readable");
        let mut text = String::new();
        archive
            .by_name(BUNDLE_MANIFEST)
            .expect("bundle.json entry")
            .read_to_string(&mut text)
            .expect("read manifest");
        let parsed: BookBundle = serde_json::from_str(&text).expect("manifest parses");
        assert_eq!(parsed.task.id, "t1");
        assert_eq!(parsed.task.total_chunks, 3);
        let names: Vec<String> = (0..archive.len())
            .filter_map(|i| archive.by_index(i).ok().map(|e| e.name().to_string()))
            .collect();
        assert!(names.contains(&"artifacts/translated.md".to_string()), "{:?}", names);
        assert!(names.contains(&"artifacts/packed/book_zh.epub".to_string()), "{:?}", names);
        drop(archive);
        assert_eq!(
            title_from_markdown(&dir.join("translated.md")).as_deref(),
            Some("测试之书")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
