mod agent_registry;
mod article_policy;
mod commands;
mod document;
mod error;
mod language_pair;
mod llm;
mod ocr;
mod pipeline;
mod glossary_store;
mod skill_library;
mod translation_validation;
mod tts;
mod word_levels;

use commands::{
    add_bookmark, add_toc_entry,
            get_page_preview, add_note, add_vocab_card, ai_assist_paragraph, ai_assist_paragraph_streaming, cancel_translation_task,
    capture_web_article, clear_runtime_notices, delete_task, export_anki_cards,
    export_backup_zip, export_reviewed_markdown, extract_difficult_words, get_reader_state, get_runtime_config,
    get_runtime_notices, get_skill_library_entry, get_task_status, export_book_bundle, import_pdf, import_translated_book,
    import_wordlist, wordlist_status, delete_wordlist,
    list_agents_registry, list_book_profiles, list_due_vocab_cards, list_toc_edit_items, list_due_vocab_cards_all,
    list_glossary_decks, list_glossary_entries, save_artifact_glossary_overrides, create_glossary_deck, rename_glossary_deck,
    delete_glossary_deck, toggle_glossary_deck, save_glossary_deck_entries,
    list_reader_fonts, import_reader_font, delete_reader_font,
    list_reader_states,
    add_vocab_notebook_entries, create_vocab_notebook, delete_vocab_notebook,
    create_skill_entry, delete_skill_entry, list_review_segments, list_route_pools, run_review_checks, set_toc_entry_page, list_skill_library_entries, list_tasks,
    list_vocab_notebooks, remove_bookmark, remove_note, remove_vocab_notebook_entry,
    rename_vocab_notebook, review_vocab_notebook_entry,
    list_tts_voices, log_reader_activity, lookup_word_level, lookup_words_batch, lookup_words_info_batch, mark_word_status,
    pause_translation_task, prepare_document_reader, prepare_epub_reader, preview_agent_prompt,
    probe_epub, read_bilingual_pairs, read_cover_data_url, read_epub_metadata, read_reader_toc, read_translated_file,
    remove_toc_entry, remove_vocab_card, rename_toc_entry, restore_backup_zip,
    resume_translation_task, retry_translation_task, review_vocab_card, save_reader_progress, save_review_edit, save_route_pools, save_user_glossary,
    save_word_definition, save_vocab_card_note, seed_book_vocab_all, set_book_favorite, set_book_folder, set_book_tags, sync_wordwise_vocab,
    pool_route_add_key, pool_route_remove_key, save_vocab_notebook_entry_note,
    search_epub_reader_content, start_translation, synthesize_tts, test_provider_connection, translate_epub_chapter_once,
    unmark_word_status, update_note, update_runtime_config, update_skill_library_entry, AppState,
};
use tauri::{Emitter, Manager};

/// 崩溃取证日志目录（%LOCALAPPDATA%\com.musereader.app\logs）：
/// 闪退无现场时，这里是唯一的证据来源——panic 全文与堆栈落盘于此。
fn log_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|dir| std::path::PathBuf::from(dir).join("com.musereader.app").join("logs"))
}

fn append_run_log(line: &str) {
    let Some(dir) = log_dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("run.log");
    /* 体积上限（4MB）：崩溃栈每条约几十 KB，超限轮转为 run.log.old——
       取证日志只保留最近一段，防长期运行无限膨胀。 */
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 4 * 1024 * 1024 {
            let _ = std::fs::rename(&path, dir.join("run.log.old"));
        }
    }
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| std::io::Write::write_all(&mut file, line.as_bytes()));
}

/// 全局 panic 钩子：默认钩子只写 stderr（发布版无控制台，信息蒸发），
/// 此处先把 panic 位置、载荷与强制采集的堆栈追加进崩溃日志再走默认流程。
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<非字符串 panic 载荷>".to_string());
        let location = info
            .location()
            .map(|loc| format!("{}:{}", loc.file(), loc.line()))
            .unwrap_or_else(|| "<未知位置>".to_string());
        let backtrace = std::backtrace::Backtrace::force_capture();
        append_run_log(&format!(
            "[{}] PANIC pid={} at {location}: {payload}\n{backtrace}\n",
            chrono::Utc::now().to_rfc3339(),
            std::process::id(),
        ));
        default_hook(info);
    }));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_hook();
    append_run_log(&format!(
        "[{}] START pid={} build={}\n",
        chrono::Utc::now().to_rfc3339(),
        std::process::id(),
        if cfg!(debug_assertions) { "debug" } else { "release" },
    ));
    let mut app_state = AppState::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 任务事件 sink：所有任务状态变更通过 `task-progress` / `task-deleted`
            // 推送到前端，替代前端 1s 轮询（前端仍保留低频兜底拉取）。
            let handle = app.handle().clone();
            app_state.set_task_event_sink(std::sync::Arc::new(
                move |event: &str, payload: String| {
                    if let Err(error) = handle.emit(event, payload) {
                        eprintln!("task event emit failed: event={event} error={error}");
                    }
                },
            ));
            // 启动装配：从磁盘用户词包重建活跃词库索引（无词包则为空表）。
            commands::wordlist::rebuild_active_index(&app_state.wordlists_dir);
            app.manage(app_state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            start_translation,
            translate_epub_chapter_once,
            import_pdf,
            import_translated_book,
            export_book_bundle,
            get_task_status,
            list_tasks,
            delete_task,
            retry_translation_task,
            cancel_translation_task,
            pause_translation_task,
            resume_translation_task,
            read_translated_file,
            read_cover_data_url,
            read_epub_metadata,
            read_bilingual_pairs,
            prepare_document_reader,
            prepare_epub_reader,
            search_epub_reader_content,
            get_runtime_config,
            get_runtime_notices,
            clear_runtime_notices,
            update_runtime_config,
            list_skill_library_entries,
            list_agents_registry,
            get_skill_library_entry,
            update_skill_library_entry,
            create_skill_entry,
            delete_skill_entry,
            preview_agent_prompt,
            get_reader_state,
            list_reader_states,
            read_reader_toc,
            save_reader_progress,
            log_reader_activity,
            add_bookmark,
            add_note,
            remove_bookmark,
            remove_note,
            probe_epub,
            list_tts_voices,
            synthesize_tts,
            lookup_word_level,
            lookup_words_batch,
            lookup_words_info_batch,
            extract_difficult_words,
            import_wordlist,
            wordlist_status,
            delete_wordlist,
            mark_word_status,
            unmark_word_status,
            save_word_definition,
            update_note,
            ai_assist_paragraph,
            ai_assist_paragraph_streaming,
            add_vocab_card,
            seed_book_vocab_all,
            sync_wordwise_vocab,
            save_vocab_card_note,
            remove_vocab_card,
            list_due_vocab_cards,
            list_due_vocab_cards_all,
            review_vocab_card,
            save_vocab_notebook_entry_note,
            export_anki_cards,
            list_review_segments,
            run_review_checks,
            save_review_edit,
            export_reviewed_markdown,
            capture_web_article,
            list_glossary_entries,
            list_book_profiles,
            export_backup_zip,
            restore_backup_zip,
            list_toc_edit_items,
            rename_toc_entry,
            remove_toc_entry,
            add_toc_entry,
            set_toc_entry_page,
            get_page_preview,
            set_book_favorite,
            set_book_folder,
            set_book_tags,
            save_user_glossary,
            save_artifact_glossary_overrides,
            list_reader_fonts,
            import_reader_font,
            delete_reader_font,
            list_glossary_decks,
            create_glossary_deck,
            rename_glossary_deck,
            delete_glossary_deck,
            toggle_glossary_deck,
            save_glossary_deck_entries,
            list_route_pools,
            save_route_pools,
            pool_route_add_key,
            pool_route_remove_key,
            list_vocab_notebooks,
            create_vocab_notebook,
            delete_vocab_notebook,
            rename_vocab_notebook,
            add_vocab_notebook_entries,
            remove_vocab_notebook_entry,
            review_vocab_notebook_entry,
            test_provider_connection,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
    /* 走到这里 = 事件循环正常退出（关闭窗口）。若日志里只有 START 没有
       EXIT 也没有 PANIC，则该次闪退不是 Rust panic——往原生层排查
       （WebView2 进程崩溃、访问违规等），为下一步取证提供方向。 */
    append_run_log(&format!(
        "[{}] EXIT pid={}\n",
        chrono::Utc::now().to_rfc3339(),
        std::process::id(),
    ));
}
