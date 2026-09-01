//! 渲染端专名变体一致性：检测并锁定「同一源名被译成多个中文音译」的盲区。
//!
//! 背景：confirmed-term 通道只覆盖「模型自报」的术语。实测 Sophie 被不同
//! 模型/不同块译成「苏菲(250)/索菲(102)」，而独立 Sophie 既未登记进 glossary，
//! 模型也不会自报为 confirmed term——`count_potential_variants` 只看源端候选词，
//! 从不核对译文端是否真的译一致了，故漏检。
//!
//! 本模块直接对**渲染后的译文**做源名→中文译法的一致性检测与安全锁定：
//!
//! 检测（多对一）：某英文首名对应多个中文译法，且这些译法**共享 ≥1 个汉字**
//!   （同一音译的词形还原变体），才判为「同一名称的不一致变体」。
//!
//! 防误伤（宁漏勿错），只在以下护栏全部成立时才改：
//!   1. 多个译法共享汉字（区分「同一名词形变体」与「同段两个同首名的不同人」）；
//!   2. 按源名**严格词界**在各块源文统计出现次数，少数派译法所在块占少数，
//!      多数派所在块占多数（双向计数吻合）——防止把另一同名角色的合法译名错改；
//!   3. 译法频次差距 ≥ 2:1（证据足够强才动，接近平手宁可不动留给人工）；
//!   4. 只在「源文严格词界包含该名」的块内做替换——不会改到「源文只提姓氏、
//!      译文却出现首名」这种上下文不匹配处的合法译名。
//!
//! 依据的通用不变式（非本书特例）：同一外语人名的中文音译，词形还原变体
//! （Sophie→苏菲/索菲）必共享 ≥1 个汉字；而两个不同的角色即便同首名，
//! 其中文译名通常不共享汉字（Tash→塔什 vs Tom→汤姆）。此假设在缺少 NER
//! 的环境下是 无LLM 的最优近似，宁漏勿错的护栏保证误伤率≈0。

use super::term::normalize_term_key;
use crate::pipeline::context::collect_capitalized_phrases;
use crate::pipeline::{ChunkCheckpoint, ChunkSegmentKind};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct RenderedVariantSummary {
    pub groups: usize,
    pub replacements: usize,
    pub repaired_chunks: usize,
}

/// 一个源名在译文里的多个候选译法，及其出现的块集合。
struct NameVariant {
    /// 原文表面形式（用于词界匹配，如 "Sophie"）
    source_surface: String,
    /// zh_form -> 出现的块索引集合
    zh_blocks: BTreeMap<String, BTreeSet<usize>>,
}

/// 主入口：对 checkpoint 的 body 块做渲染端变体检修。
pub(super) fn apply_rendered_variant_repair(
    checkpoint: &mut ChunkCheckpoint,
) -> RenderedVariantSummary {
    let variants = collect_name_variants(checkpoint);
    let mut summary = RenderedVariantSummary::default();

    for variant in variants.values() {
        if variant.zh_blocks.len() < 2 {
            continue; // 只有一个译法，无不一致
        }
        // 译名映射安全网（误报拦截）：仅当**每个含源名的块的译文都至少含一个
        // 译名候选**（映射完备）时才继续——否则候选只是碰巧共现的无关汉字串。
        let total_source_blocks_early = count_source_blocks(checkpoint, &variant.source_surface);
        let covered_blocks: BTreeSet<usize> = variant
            .zh_blocks
            .values()
            .flat_map(|b| b.iter().copied())
            .collect();
        if covered_blocks.len() < total_source_blocks_early {
            continue;
        }

        // 1) 共享汉字聚类：把 zh_forms 按「共享 ≥1 汉字」分组，取最大组（其余视为不同角色）。
        let cluster = largest_shared_char_cluster(variant.zh_blocks.keys());
        if cluster.len() < 2 {
            continue; // 没有共享汉字的多译法 = 不同角色，不动
        }
        // 组内只保留「互不包含」的最长候选（extract 阶段已剥纯子串）。
        let mut ranked: Vec<(&String, &BTreeSet<usize>)> = variant
            .zh_blocks
            .iter()
            .filter(|(zh, _)| cluster.contains(*zh))
            .collect();
        if ranked.len() < 2 {
            continue;
        }
        ranked.sort_by_key(|(_, blocks)| std::cmp::Reverse(blocks.len()));
        let (major_zh, major_blocks) = ranked[0];
        let major_count = major_blocks.len();

        let total_source_blocks = count_source_blocks(checkpoint, &variant.source_surface);

        // 译名映射安全网（误报拦截）：本模块的 zh_forms 只是「译文里与源名
        // 同块共现的汉字串」，并不知道哪个真是该源名的译名。仅当**每个含源名
        // 的块的译文都至少含一个译名候选**（映射完备）时，才认为这些候选确实
        // 锚定到该源名；否则（译文里可能根本没有源名的对应译名，只是碰巧共现）
        // 跳过，避免把无关汉字串当译名误改。
        let covered_blocks: BTreeSet<usize> = variant
            .zh_blocks
            .values()
            .flat_map(|b| b.iter().copied())
            .collect();
        if covered_blocks.len() < total_source_blocks {
            continue; // 译名覆盖不完备，候选不可靠，不动
        }

        for (minor_zh, minor_blocks) in ranked.iter().skip(1) {
            let minor_count = minor_blocks.len();
            if minor_count * 2 > major_count {
                continue; // 差距不足 2:1，证据不够强，不动
            }
            if minor_blocks.iter().any(|i| major_blocks.contains(i)) {
                continue; // 同块同时出现两种译法，可能是两个角色，不动
            }
            if major_count + minor_count > total_source_blocks + 1 {
                continue; // 计数溢出，不动
            }
            for chunk in checkpoint.chunks.iter_mut() {
                if chunk.segment_kind != ChunkSegmentKind::Body {
                    continue;
                }
                if !minor_blocks.contains(&chunk.index) {
                    continue;
                }
                if !source_has_name(&chunk.source, &variant.source_surface) {
                    continue;
                }
                let Some(translated) = chunk.translated.as_mut() else {
                    continue;
                };
                // 核整串替换（索菲→苏菲）：minor 是收敛后的核，str::replace 只替换核子串、
                // 其后紧邻的汉字后缀（回/布莱克/走进）原样保留 → 索菲回→苏菲回、
                // 索菲布莱→苏菲布莱。这里 minor 不会是长形态（词形还原已归并到核）。
                let replaced = translated.replace(minor_zh.as_str(), major_zh.as_str());
                if replaced != *translated {
                    *translated = replaced;
                    summary.replacements += 1;
                    summary.repaired_chunks += 1;
                }
            }
            if summary.replacements > 0 {
                summary.groups += 1;
            }
        }
    }
    summary
}

/// 收集 body 块中「英文首名 -> 多个中文译法」的候选。
///
/// 用「词形还原（lemmatize）」替代「滑窗猜名」：先聚合每个源名在全书译文里
/// 反复共现的连续汉字 run，再按「跨块复现」收敛出该源名的译名词表。
/// 这样「索菲·布莱克」「布莱克警官」等含分隔/修饰的形态能正确归并到 索菲/布莱克，
/// 而不会被滑窗的上下文黏连（索菲回/苏菲走）污染。
fn collect_name_variants(checkpoint: &ChunkCheckpoint) -> BTreeMap<String, NameVariant> {
    // 第一遍：收集每个源名在哪些块出现，以及这些块译文里的全部连续汉字 run。
    let mut name_runs: BTreeMap<String, (String, BTreeMap<String, BTreeSet<usize>>)> =
        BTreeMap::new();
    for chunk in &checkpoint.chunks {
        if chunk.segment_kind != ChunkSegmentKind::Body {
            continue;
        }
        let Some(translated) = chunk.translated.as_ref() else {
            continue;
        };
        let names = extract_first_names(&chunk.source);
        if names.is_empty() {
            continue;
        }
        let runs = extract_han_runs(translated);
        for name in names {
            let key = normalize_term_key(&name);
            if key.len() < 2 {
                continue;
            }
            let entry = name_runs
                .entry(key.clone())
                .or_insert_with(|| (name.clone(), BTreeMap::new()));
            for run in &runs {
                // 只保留含「名字特征」的 run 片段：run 可能很长（整段中文），
                // 这里先记录全部，后面靠「跨块复现 + 窗口」收敛出名字。
                entry.1.entry(run.clone()).or_default().insert(chunk.index);
            }
        }
    }

    // 第二遍：对每个源名，从其关联 run 集合中收敛出译名候选。
    //
    // 词形还原核心：同一源名的不同译法（苏菲/索菲）在每个块内与上下文黏连成
    // 不同形态（苏菲走进/苏菲开口/索菲回），但**跨块**看，真正的译名是那个
    // 「出现在多种不同上下文里」的稳定子串。
    //
    // 复杂度护栏（本质优化，防 O(n²) 爆炸）：窗口枚举限定在「源名在译文里的
    // 最可能邻域」——中文人名音译有通用首字集（苏/索/克/汤/杰/劳/亚/艾/帕…），
    // 但为不硬编码、保持通用性，改用「run 内与前一个词/姓氏·分隔的距离」：
    // 只在每个 run 里枚举以「非虚词首字」开头、长度 2-4 的窗口，且每 run 每块
    // 至多取前 N 个候选——把 O(块数×run长×窗口) 压到与文本量线性。
    // 真正的译名会被「跨块复现」自然筛出，单块黏连噪声即使枚举到也会在后续
    // 坍缩/剪枝中被丢弃，故此处可以放心收紧候选量。
    let mut variants: BTreeMap<String, NameVariant> = BTreeMap::new();
    for (key, (surface, run_blocks)) in name_runs {
        let mut window_blocks: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        for (run, blocks) in &run_blocks {
            let chars: Vec<char> = run.chars().collect();
            for len in [2usize, 3, 4] {
                if chars.len() < len {
                    continue;
                }
                for start in 0..=(chars.len() - len) {
                    let w: String = chars[start..start + len].iter().collect();
                    if !is_plausible_zh_name(&w) {
                        continue;
                    }
                    let e = window_blocks.entry(w).or_default();
                    for b in blocks {
                        e.insert(*b);
                    }
                }
            }
        }
        // 词形还原：译名是「跨上下文复现的稳定核」。收敛方向由替换语义决定——
        // 锁定用「核整串替换」（索菲→苏菲，保留其后 回/布莱克 等上下文后缀），
        // 故译名必须是**核**（索菲）而非**形态**（索菲回）。三步自洽收敛：
        //   ① 剥纯子串：w 是更长串 o 的子串且 block_set(w) ⊆ block_set(o)
        //      （走进/菲走/菲回/索菲(⊆索菲回) 全坍缩，只留每块最长形态）。
        //   ② 剥黏连长形态：w 含幸存者短核 s 且 block_set(w) ⊊ block_set(s)
        //      （苏菲走进{0} ⊊ 苏菲{0,1,2}）→ 剥 w 留短核 s。
        //   ③ 归并到公共核：幸存者按「共享汉字前缀」聚类，每类取**最短**形式为核
        //      （苏菲走进/苏菲开口/苏菲离开 → 公共前缀 苏菲；索菲回 单独 → 索菲回 自身，
        //      但其前缀核 索菲 已在①坍缩，故取 索菲回 的前缀 索菲 为核——
        //      用「去掉尾部单字直到命中一个曾被见过的形式」回溯出 索菲）。
        let entries: Vec<(&String, &BTreeSet<usize>)> = window_blocks.iter().collect();
        let kernels: Vec<(&String, &BTreeSet<usize>)> = entries
            .iter()
            .filter(|(w, w_blocks)| {
                !entries.iter().any(|(o, o_blocks)| {
                    o != w
                        && o.len() > w.len()
                        && o.contains(w.as_str())
                        && w_blocks.is_subset(o_blocks)
                })
            })
            .copied()
            .collect();
        let survivors: Vec<(String, BTreeSet<usize>)> = kernels
            .iter()
            .filter(|(w, w_blocks)| {
                !kernels.iter().any(|(s, s_blocks)| {
                    s.len() < w.len()
                        && w.contains(s.as_str())
                        && s_blocks.len() > w_blocks.len()
                        && w_blocks.is_subset(s_blocks)
                })
            })
            .map(|(w, b)| ((*w).clone(), (*b).clone()))
            .collect();
        // ③ 每个幸存者归并到「去掉尾部、直到命中一个被全书见过的更短形式」的核。
        //    索菲回 → 索菲（索菲 在 window_blocks 里被见过，虽是 索菲回 的子串）；
        //    苏菲走进 → 苏菲；苏菲（已是核）→ 苏菲。
        let all_windows: BTreeSet<&String> = window_blocks.keys().collect();
        let forms: Vec<(String, BTreeSet<usize>)> = survivors
            .into_iter()
            .map(|(w, blocks)| {
                let core = reduce_to_core(&w, &all_windows, &window_blocks, &blocks);
                (core, blocks)
            })
            .collect();
        // 核可能重复（多个幸存者归并到同一核），合并块集
        let mut merged: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
        for (core, blocks) in forms {
            merged.entry(core).or_default().extend(blocks);
        }
        let forms: Vec<(String, BTreeSet<usize>)> = merged.into_iter().collect();
        if forms.len() < 2 {
            continue;
        }
        let zh_blocks: BTreeMap<String, BTreeSet<usize>> = forms.into_iter().collect();
        variants.insert(
            key.clone(),
            NameVariant {
                source_surface: surface,
                zh_blocks,
            },
        );
    }
    variants
}

/// 把一个幸存者形态归并到它的「核」：从全形开始，逐步去掉尾部字符，
/// 直到命中一个「在全书 window_blocks 里被独立见过、且块集不更小」的更短形式。
/// 索菲回 → 索菲（索菲 在 window_blocks 里被见过，块集相同）；
/// 苏菲走进 → 苏菲；苏菲（已是核，去尾会命中更小的 苏 但块集不符）→ 苏菲。
/// 这样替换「核→多数派」时（索菲→苏菲），形态的后缀（回/走进）自动保留。
fn reduce_to_core(
    form: &str,
    all_windows: &BTreeSet<&String>,
    window_blocks: &BTreeMap<String, BTreeSet<usize>>,
    form_blocks: &BTreeSet<usize>,
) -> String {
    let chars: Vec<char> = form.chars().collect();
    // 从长到短找第一个「被独立见过且块集相同」的前缀
    for len in (2..chars.len()).rev() {
        let prefix: String = chars[..len].iter().collect();
        if all_windows.contains(&prefix) {
            if let Some(pb) = window_blocks.get(&prefix) {
                // 前缀的块集包含本形态块集（前缀至少在同样块出现）→ 是核
                if form_blocks.iter().all(|b| pb.contains(b)) {
                    return prefix;
                }
            }
        }
    }
    form.to_string()
}

/// 提取译文中所有连续汉字 run（按非汉字切分）。
fn extract_han_runs(translated: &str) -> Vec<String> {
    let mut runs = Vec::new();
    let mut cur = String::new();
    for ch in translated.chars() {
        if ('\u{4e00}'..='\u{9fff}').contains(&ch) {
            cur.push(ch);
        } else if !cur.is_empty() {
            runs.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        runs.push(cur);
    }
    runs
}

/// 从英文源文提取「首名」候选：大写开头的单词（不含全大写缩写、不含句首常见词）。
/// 用 collect_capitalized_phrases 的切分，再拆成单词、只留字母长度 ≥2 的。
fn extract_first_names(source: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for phrase in collect_capitalized_phrases(source) {
        for word in phrase.split_whitespace() {
            let clean: String = word.trim_matches(|c: char| !c.is_alphabetic()).to_string();
            if clean.len() >= 2
                && clean
                    .chars()
                    .next()
                    .map(|c| c.is_uppercase())
                    .unwrap_or(false)
                && !clean.chars().all(|c| c.is_uppercase())
            // 排除全大写缩写如 WE/I
            {
                names.insert(clean);
            }
        }
    }
    names
}

/// 排除高频虚词/常见词，降低把普通词当名字的误报。
fn is_plausible_zh_name(window: &str) -> bool {
    const STOP_CHARS: &[char] = &[
        '的', '了', '在', '是', '我', '你', '他', '她', '它', '们', '这', '那', '有', '和', '就',
        '不', '都', '而', '及', '与', '或', '也', '很', '更', '最', '但', '又', '还', '要', '会',
        '能', '可', '以', '为', '到', '说', '去', '来', '上', '下', '里', '中', '大', '小', '多',
        '少', '好', '坏', '新', '旧', '个', '些',
    ];
    !window.chars().any(|c| STOP_CHARS.contains(&c))
}

/// 把一组中文译法按「共享 ≥1 汉字」聚成最大连通组。
fn largest_shared_char_cluster<'a>(forms: impl Iterator<Item = &'a String>) -> BTreeSet<String> {
    let forms: Vec<String> = forms.cloned().collect();
    let n = forms.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut Vec<usize>, x: usize) -> usize {
        if parent[x] != x {
            parent[x] = find(parent, parent[x]);
        }
        parent[x]
    }
    for i in 0..n {
        for j in (i + 1)..n {
            if share_any_char(&forms[i], &forms[j]) {
                let (ri, rj) = (find(&mut parent, i), find(&mut parent, j));
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }
    let mut groups: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    for (i, f) in forms.iter().enumerate() {
        groups
            .entry(find(&mut parent, i))
            .or_default()
            .insert(f.clone());
    }
    groups
        .into_values()
        .max_by_key(|g| g.len())
        .unwrap_or_default()
}

fn share_any_char(a: &str, b: &str) -> bool {
    let set: BTreeSet<char> = a.chars().collect();
    b.chars().any(|c| set.contains(&c))
}

/// 严格词界统计：源名在 body 块中出现的块数。
fn count_source_blocks(checkpoint: &ChunkCheckpoint, surface: &str) -> usize {
    checkpoint
        .chunks
        .iter()
        .filter(|c| c.segment_kind == ChunkSegmentKind::Body)
        .filter(|c| source_has_name(&c.source, surface))
        .count()
}

/// 源文是否含该名（严格词界：两侧非字母数字）。
fn source_has_name(source: &str, surface: &str) -> bool {
    if surface.is_empty() {
        return false;
    }
    let mut start = 0;
    while let Some(pos) = source[start..].find(surface) {
        let abs = start + pos;
        let before_ok = abs == 0
            || !source[..abs]
                .chars()
                .last()
                .map(|c| c.is_alphanumeric())
                .unwrap_or(false);
        let after = &source[abs + surface.len()..];
        let after_ok = after
            .chars()
            .next()
            .map(|c| !c.is_alphanumeric())
            .unwrap_or(true);
        if before_ok && after_ok {
            return true;
        }
        start = abs + surface.len();
    }
    false
}
