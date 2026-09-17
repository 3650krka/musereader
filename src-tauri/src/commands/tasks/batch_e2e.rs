//! 一次性 headless 批跑入口（仅 `#[cfg(test)]` 编译，不进生产二进制）。
//! 与原生命令同入口：start_translation_inner → 完成后若质量闸不过，用
//! retry_translation_task_inner 做**同任务原生补译**（复用 artifact 目录与检查点，
//! 管线自动 invalidate 未译/污染块，只重译坏块）。
//! 号池：worker 数放开，交给池内 water-filling + 学习画像自适应分配。

#![cfg(test)]

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::{lifecycle, operations, TranslationTask};
use crate::commands::{files, wordlist, AppState};

const EN_DIR: &str = r"D:\Projects\musetranslate\click\test_file\blackwood-stories\en";
const ZH_DIR: &str = r"D:\Projects\musetranslate\click\test_file\blackwood-stories\zh";
const PROGRESS_LOG: &str = r"D:\Projects\musetranslate\click\_local\_batch_rust.log";
const PER_TASK_TIMEOUT: Duration = Duration::from_secs(90 * 60);
/// 并发 worker：交给号池 water-filling 在活路由间分配（画像会自适应收敛）
const MAX_WORKERS: u8 = 6;
/// 单篇最多原生补译轮次（同任务重试）
const MAX_NATIVE_RETRIES: u32 = 3;

fn say(msg: &str) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("t{} {}", secs % 100_000, msg);
    println!("{line}");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(PROGRESS_LOG) {
        use std::io::Write;
        let _ = writeln!(f, "{line}");
    }
}

/// 残留英文整段（管线失败分块回退原文的形态）
fn has_residual_english_block(content: &str) -> bool {
    content.split('\n').any(|para| {
        let cjk = para.chars().filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c)).count();
        let en = para
            .split(|c: char| !c.is_ascii_alphabetic())
            .filter(|w| !w.is_empty())
            .count();
        en > 30 && en > cjk * 3
    })
}

fn quality(content: &str) -> Option<(f64, bool)> {
    if content.trim().chars().count() <= 40 {
        return None;
    }
    let cjk = content.chars().filter(|c| ('\u{4e00}'..='\u{9fff}').contains(c)).count();
    let latin = content
        .split(|c: char| !(c.is_ascii_alphabetic() || c == '\'' || c == '\u{2019}'))
        .filter(|w| w.len() > 1)
        .count();
    let ratio = cjk as f64 / (cjk + latin).max(1) as f64;
    Some((ratio, has_residual_english_block(content)))
}

async fn wait_terminal(state: &AppState, task_id: &str, name: &str, t0: Instant) -> Option<TranslationTask> {
    let mut last = String::new();
    while t0.elapsed() < PER_TASK_TIMEOUT {
        match lifecycle::task_snapshot(state, task_id) {
            Ok(t) => {
                let key = format!("{:?}/{:?} {}%", t.status, t.phase, t.progress);
                if key != last {
                    say(&format!("  {name} [{}s] {key}", t0.elapsed().as_secs()));
                    last = key;
                }
                let dbg = format!("{:?}", t.status);
                if dbg == "Completed" || dbg == "Failed" {
                    return Some(t);
                }
            }
            Err(e) => say(&format!("POLL-ERR {name} {e:?}")),
        }
        tokio::time::sleep(Duration::from_secs(6)).await;
    }
    say(&format!("TIMEOUT {name}"));
    None
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "手动批跑入口，非回归测试"]
async fn cas_batch_translate() {
    let en = PathBuf::from(EN_DIR);
    let zh = PathBuf::from(ZH_DIR);
    fs::create_dir_all(&zh).expect("create zh dir");

    let mut state = AppState::new();
    state.set_task_event_sink(std::sync::Arc::new(|event: &str, payload: String| {
        let head: String = payload.chars().take(100).collect();
        say(&format!("EVENT {event} {head}"));
    }));
    let rebuilt = wordlist::rebuild_active_index(&state.wordlists_dir);
    say(&format!("wordlist rebuilt entries={rebuilt}"));

    let mut inputs: Vec<PathBuf> = fs::read_dir(&en)
        .expect("read en dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    inputs.sort();
    // 指定单篇模式：BATCH_ONLY=<文件名子串> 只跑匹配的篇目
    if let Ok(only) = std::env::var("BATCH_ONLY") {
        if !only.is_empty() {
            inputs.retain(|p| p.file_name().unwrap().to_string_lossy().contains(&only));
            say(&format!("BATCH_ONLY={only} matched={}", inputs.len()));
        }
    }
    // 单篇验证模式：BATCH_LIMIT=N 只跑前 N 篇（默认 0 = 全部）
    let limit: usize = std::env::var("BATCH_LIMIT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    if limit > 0 {
        inputs.truncate(limit);
        say(&format!("BATCH_LIMIT={limit}"));
    }

    let total = inputs.len();
    let mut written = 0usize;
    for src in &inputs {
        let name = src.file_name().unwrap().to_string_lossy().to_string();
        let dst = zh.join(&name);
        if dst.exists() && fs::metadata(&dst).map(|m| m.len()).unwrap_or(0) > 120 {
            say(&format!("SKIP(done) {name}"));
            continue;
        }
        say(&format!("START {name}"));
        let t0 = Instant::now();
        let src_str = src.to_string_lossy().to_string();

        // 首轮：新任务
        let mut task_id = match operations::start_translation_inner(
            &state, &src_str, "fiction", true, MAX_WORKERS, None,
        ) {
            Ok(id) => id,
            Err(e) => {
                say(&format!("START-FAIL {name} {e:?}"));
                continue;
            }
        };
        say(&format!("taskId {name} {task_id}"));

        let mut accepted = false;
        for attempt in 0..=MAX_NATIVE_RETRIES {
            let Some(task) = wait_terminal(&state, &task_id, &name, t0).await else {
                break;
            };
            let status = format!("{:?}", task.status);
            say(&format!("FINAL {name} attempt={attempt} status={status} {}s", t0.elapsed().as_secs()));

            let out = task.output_path.clone();
            let mut degraded = status != "Completed" || out.is_none();
            let mut ratio = 0.0f64;
            let mut residual = true;
            if let Some(path) = out {
                match files::read_translated_file(path).await {
                    Ok(content) => match quality(&content) {
                        Some((r, res)) => {
                            ratio = r;
                            residual = res;
                            degraded = degraded || r < 0.8 || res;
                            if !degraded {
                                fs::write(&dst, content).expect("write zh");
                                say(&format!("WROTE {name} cjk_ratio={r:.2} attempts={attempt}"));
                                written += 1;
                                accepted = true;
                            }
                        }
                        None => {
                            say(&format!("READ-EMPTY {name}"));
                        }
                    },
                    Err(e) => say(&format!("READ-FAIL {name} {e:?}")),
                }
            }
            if accepted {
                break;
            }
            // 原生补译：同任务重试 → 检查点复用，只重译坏块
            if attempt < MAX_NATIVE_RETRIES {
                match operations::retry_translation_task_inner(&state, &task_id) {
                    Ok(()) => say(&format!(
                        "NATIVE-RETRY {name} attempt={attempt} (ratio={ratio:.2} residual={residual})"
                    )),
                    Err(e) => {
                        say(&format!("RETRY-FAIL {name} {e:?}"));
                        break;
                    }
                }
            } else {
                say(&format!("UNRESOLVED {name} ratio={ratio:.2} residual={residual}"));
            }
        }
    }
    say(&format!("DONE-BATCH written={written}/{total}"));
}
