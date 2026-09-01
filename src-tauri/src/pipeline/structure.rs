use crate::error::AppError;
use crate::llm::SensenovaClient;
use crate::ocr::OcrPageLayout;
use crate::pipeline::render::authors::{
    AcademicAffiliationEntry, AcademicAuthor, AcademicFrontMatter, AcademicFrontMatterExtras,
};
use crate::pipeline::retry_async_with_observer;
use serde::{Deserialize, Serialize};
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HeaderBlockCandidate {
    pub(super) blocks: Vec<String>,
    pub(super) probe_blocks: Vec<String>,
    pub(super) body_start_block: usize,
    pub(super) boundary: HeaderBoundary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderBlockKind {
    Empty,
    Title,
    AbstractHeading,
    SectionHeading,
    AuthorLine,
    AffiliationLine,
    ContactLine,
    DoiOrJournalMeta,
    UpdateNotice,
    CopyrightOrLicense,
    BodyParagraph,
    UnknownFrontMatter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HeaderBoundary {
    FirstBodyParagraph,
    FirstSecondLevelHeading,
    DocumentEnd,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct FrontMatterExtraction {
    pub(super) title: String,
    #[serde(default)]
    pub(super) authors: Vec<FrontMatterAuthor>,
    #[serde(default)]
    pub(super) extras: Vec<String>,
    #[serde(default)]
    pub(super) affiliation_catalog: Vec<FrontMatterAffiliationEntry>,
    #[serde(default)]
    pub(super) extra_entries: Vec<FrontMatterExtraEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct FrontMatterAuthor {
    pub(super) name: String,
    #[serde(default)]
    pub(super) markers: Vec<String>,
    #[serde(default)]
    pub(super) affiliations: Vec<String>,
    pub(super) email: Option<String>,
    #[serde(default)]
    pub(super) is_corresponding: bool,
    #[serde(default)]
    pub(super) affiliation_refs: Vec<String>,
    #[serde(default)]
    pub(super) equal_contribution: Vec<String>,
    #[serde(default)]
    pub(super) author_notes: Vec<String>,
    #[serde(default)]
    pub(super) notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct FrontMatterAffiliationEntry {
    pub(super) id: String,
    pub(super) text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(super) struct FrontMatterExtraEntry {
    pub(super) raw: String,
    pub(super) kind: String,
}

pub(super) fn detect_header_blocks(markdown: &str) -> HeaderBlockCandidate {
    let blocks = split_blocks(markdown);
    let boundary_index = detect_header_boundary_index(&blocks);
    let header_only_blocks = blocks
        .iter()
        .take(boundary_index.0)
        .filter(|block| !is_update_notice_block(block))
        .cloned()
        .collect::<Vec<_>>();

    HeaderBlockCandidate {
        blocks: header_only_blocks,
        probe_blocks: blocks
            .iter()
            .skip(boundary_index.0)
            .take(4)
            .cloned()
            .collect(),
        body_start_block: boundary_index.0,
        boundary: boundary_index.1,
    }
}

pub(super) fn append_candidate_context(
    candidate: &HeaderBlockCandidate,
    label: &str,
    context_blocks: &[String],
) -> HeaderBlockCandidate {
    let mut blocks = candidate.blocks.clone();
    let context = context_blocks
        .iter()
        .map(|block| block.trim())
        .filter(|block| !block.is_empty())
        .collect::<Vec<_>>();
    if !context.is_empty() {
        blocks.push(format!("{}:\n{}", label.trim(), context.join("\n\n")));
    }
    HeaderBlockCandidate {
        blocks,
        probe_blocks: candidate.probe_blocks.clone(),
        body_start_block: candidate.body_start_block,
        boundary: candidate.boundary,
    }
}

pub(super) fn front_matter_candidate_from_layout(
    markdown: &str,
    page_layouts: &[OcrPageLayout],
) -> HeaderBlockCandidate {
    let candidate = detect_header_blocks(markdown);
    let layout_blocks = first_page_header_layout_blocks(page_layouts);
    let candidate =
        append_candidate_context(&candidate, "First-page layout context", &layout_blocks);
    let footnotes = first_page_footnote_blocks(page_layouts);
    append_candidate_context(&candidate, "First-page footnote", &footnotes)
}

pub(super) fn first_page_header_layout_blocks(page_layouts: &[OcrPageLayout]) -> Vec<String> {
    let Some(first_page) = page_layouts.first() else {
        return Vec::new();
    };
    let body_top = first_page
        .blocks
        .iter()
        .filter(|block| {
            let label = block.label.to_ascii_lowercase();
            let content = block.content.trim().to_ascii_lowercase();
            label == "paragraph_title" && matches!(content.as_str(), "abstract" | "introduction")
        })
        .filter_map(block_top)
        .min();

    let mut blocks = first_page
        .blocks
        .iter()
        .filter(|block| {
            let label = block.label.to_ascii_lowercase();
            matches!(label.as_str(), "doc_title" | "text" | "header" | "footnote")
        })
        .filter(|block| {
            body_top
                .map(|top| block_top(block).unwrap_or(i32::MAX) < top)
                .unwrap_or(true)
        })
        .filter_map(|block| {
            let content = block.content.trim();
            (!content.is_empty()).then(|| {
                (
                    block.order.unwrap_or(i32::MAX),
                    block_top(block).unwrap_or(i32::MAX),
                    content.to_string(),
                )
            })
        })
        .collect::<Vec<_>>();
    blocks.sort_by_key(|(order, top, _)| (*order, *top));
    blocks
        .into_iter()
        .map(|(_, _, content)| content)
        .take(16)
        .collect()
}

fn block_top(block: &crate::ocr::OcrLayoutBlock) -> Option<i32> {
    block.bbox.get(1).copied()
}

pub(super) fn first_page_footnote_blocks(page_layouts: &[OcrPageLayout]) -> Vec<String> {
    let Some(first_page) = page_layouts.first() else {
        return Vec::new();
    };
    let mut footnotes = first_page
        .blocks
        .iter()
        .filter(|block| block.label.eq_ignore_ascii_case("footnote"))
        .filter_map(|block| {
            let content = block.content.trim();
            (!content.is_empty()).then(|| (block.order.unwrap_or(i32::MAX), content.to_string()))
        })
        .collect::<Vec<_>>();
    footnotes.sort_by_key(|(order, _)| *order);
    footnotes.into_iter().map(|(_, content)| content).collect()
}

pub(super) fn candidate_has_author_signal(candidate: &HeaderBlockCandidate) -> bool {
    candidate.blocks.iter().any(|block| {
        let lower = block.to_ascii_lowercase();
        block.contains('@')
            || block.contains(" $ ^{")
            || block.contains(r"\textcircled")
            || lower.contains("first-page footnote")
            || lower.contains("corresponding author")
            || lower.contains("e-mail")
            || lower.contains("affiliation")
    })
}

pub(super) async fn extract_front_matter_with_llm(
    llm_client: &SensenovaClient,
    candidate: &HeaderBlockCandidate,
) -> Result<FrontMatterExtraction, AppError> {
    let effective_candidate = if header_candidate_needs_boundary_assist(candidate) {
        match refine_header_candidate_with_llm(llm_client, candidate).await {
            Ok(refined) => refined,
            Err(_) => candidate.clone(),
        }
    } else {
        candidate.clone()
    };
    let prompt = build_front_matter_prompt(&effective_candidate);
    let mut last_error = None;
    for delay_ms in [0u64, 2_500u64] {
        if delay_ms > 0 {
            sleep(Duration::from_millis(delay_ms)).await;
        }
        match llm_client.review_translation_issue(&prompt).await {
            Ok(response) => {
                let parsed = parse_front_matter_response(&response)?;
                return Ok(enrich_front_matter_from_candidate(
                    parsed,
                    &effective_candidate,
                ));
            }
            Err(error) => last_error = Some(AppError::from_provider_error(error)),
        }
    }

    if let Some(fallback) = extract_front_matter_with_heuristics(&effective_candidate) {
        return Ok(fallback);
    }

    Err(last_error.unwrap_or_else(|| {
        AppError::internal("front matter extraction failed without a recoverable result")
    }))
}

pub(super) async fn translate_front_matter_with_llm(
    llm_client: &SensenovaClient,
    front_matter: &AcademicFrontMatter,
) -> Result<AcademicFrontMatter, AppError> {
    if front_matter
        .title
        .as_deref()
        .is_none_or(|title| title.trim().is_empty())
    {
        return Ok(front_matter.clone());
    }

    let prompt = build_front_matter_translation_prompt(front_matter)?;
    let response = review_issue_with_rate_limit_retry(llm_client, &prompt).await?;
    let mut translated = parse_translated_front_matter_response(&response)?;
    if front_matter_needs_translation_repair(front_matter, &translated) {
        let repair_prompt =
            build_front_matter_translation_repair_prompt(front_matter, &translated)?;
        let repaired_response =
            review_issue_with_rate_limit_retry(llm_client, &repair_prompt).await?;
        let repaired = parse_translated_front_matter_response(&repaired_response)?;
        if !front_matter_needs_translation_repair(front_matter, &repaired) {
            translated = repaired;
        }
    }

    Ok(AcademicFrontMatter {
        title: translated.title,
        authors: translated.authors,
        extras: translated.extras,
        affiliation_catalog: translated.affiliation_catalog,
        extra_entries: translated.extra_entries,
    })
}

// finalize 阶段 LLM 调用此前无 429 退避：单次 review_translation_issue 遇限流直接
// panic（per-fall 复测实证）。此处补与主翻译/residual 一致的重试：retry_async_with_observer
// 内部对 rate-limit/5xx/超时做退避（compute_retry_delay 对 429 用 max(retry_after,60s,stepped)），
// 对 400 等非重试错误直接失败。取消语义：无独立 token，用恒定 false（finalize 在
// finalize_translation_outputs 入口处已 ensure_not_cancelled）。
async fn review_issue_with_rate_limit_retry(
    llm_client: &SensenovaClient,
    prompt: &str,
) -> Result<String, AppError> {
    retry_async_with_observer(
        3,
        Duration::from_secs(2),
        || false,
        |_attempt| {
            let client = llm_client.clone();
            let prompt = prompt.to_string();
            async move {
                client
                    .review_translation_issue(&prompt)
                    .await
                    .map_err(AppError::from_provider_error)
            }
        },
        |_attempt, error, delay| {
            // 记录 finalize 阶段 LLM 重试（含超时/限流），便于复测归因"是否换了路由"。
            // reserve_route 在每次 attempt 都会按健康度×速度重选路由——连续失败的
            // 路由健康度被打低，第 2/3 次尝试自然切到更优路由。
            let _ = (error, delay);
        },
    )
    .await
}

pub(super) fn build_front_matter_prompt(candidate: &HeaderBlockCandidate) -> String {
    let header = candidate.blocks.join("\n\n");
    format!(
        "You are extracting front-matter metadata from OCR Markdown for an academic PDF.\n\
         Return JSON only. Do not add Markdown or HTML.\n\
         Use only information present in the input. Do not invent missing fields.\n\
         The title must be the article title, not a journal label, DOI, or article type.\n\
         Authors must preserve order. Include markers, affiliations, affiliation refs, email, corresponding-author status, equal-contribution notes, and author notes when present.\n\
         Treat blocks labelled 'First-page layout context' and 'First-page footnote' as author, affiliation, and contact context when they contain author lines, affiliations, equal-contribution notes, or corresponding-author details; do not classify those context blocks as body text.\n\
         If a single affiliation line contains multiple departments, institutes, universities, schools, centers, laboratories, faculties, colleges, or hospitals joined by 'and', split them only when the right side starts a new affiliation phrase, then assign them by author order if no explicit refs are present.\n\
         If affiliation text is represented once in affiliationCatalog and authors refer to it by affiliationRefs, keep that structure and do not duplicate unrelated body sentences as affiliations.\n\
         Put DOI, article type labels, update notices, journal labels, or other non-author metadata in extras and also classify them into extraEntries when possible.\n\
         Do not include copyright, open-access license, or Creative Commons boilerplate in extras.\n\
         If affiliation details are not present in the input, keep affiliations empty.\n\
         If affiliation ids or numeric markers are clear, put them into affiliationCatalog and author affiliationRefs.\n\
         Keep legacy notes for compatibility, but prefer putting richer note semantics into equalContribution and authorNotes.\n\
         JSON schema:\n\
         {{\"title\":\"...\",\"authors\":[{{\"name\":\"...\",\"markers\":[\"1\"],\"affiliations\":[\"...\"],\"affiliationRefs\":[\"1\"],\"email\":null,\"isCorresponding\":false,\"equalContribution\":[\"...\"],\"authorNotes\":[\"...\"],\"notes\":[\"...\"]}}],\"extras\":[\"...\"],\"affiliationCatalog\":[{{\"id\":\"1\",\"text\":\"...\"}}],\"extraEntries\":[{{\"raw\":\"...\",\"kind\":\"doi|receivedDate|acceptedDate|publishedDate|journalMeta|articleNote|other\"}}]}}\n\n\
         Header candidate:\n{header}"
    )
}

pub(super) fn extract_front_matter_with_heuristics(
    candidate: &HeaderBlockCandidate,
) -> Option<FrontMatterExtraction> {
    let header_markdown = candidate.blocks.join("\n\n");
    let authors = crate::pipeline::render::extract_academic_authors(&header_markdown);
    let title = detect_title_from_candidate(candidate)?;
    let extras = collect_extra_lines(candidate, &title, &authors);
    let affiliation_catalog = build_front_matter_affiliation_catalog(&authors);
    let extra_entries = classify_front_matter_extra_entries(&extras);
    let extraction = FrontMatterExtraction {
        title,
        authors: authors
            .into_iter()
            .map(|author| FrontMatterAuthor {
                name: author.name,
                markers: author.markers,
                affiliations: author.affiliations,
                affiliation_refs: author.affiliation_refs,
                email: author.email,
                is_corresponding: author.is_corresponding,
                equal_contribution: author.equal_contribution,
                author_notes: author.author_notes.clone(),
                notes: author
                    .correspondence_note
                    .into_iter()
                    .chain(author.author_notes.into_iter())
                    .collect::<Vec<_>>(),
            })
            .collect(),
        extras,
        affiliation_catalog,
        extra_entries,
    };
    let sanitized = sanitize_front_matter(extraction).ok()?;
    ((!sanitized.title.trim().is_empty()) || !sanitized.authors.is_empty()).then_some(sanitized)
}

fn detect_title_from_candidate(candidate: &HeaderBlockCandidate) -> Option<String> {
    candidate
        .blocks
        .iter()
        .map(|block| block.trim())
        .filter(|block| !block.is_empty())
        .find(|block| {
            matches!(
                classify_header_block(block),
                HeaderBlockKind::Title | HeaderBlockKind::UnknownFrontMatter
            ) && !block
                .to_ascii_lowercase()
                .starts_with("first-page footnote:")
        })
        .map(ToString::to_string)
}

fn collect_extra_lines(
    candidate: &HeaderBlockCandidate,
    title: &str,
    authors: &[AcademicAuthor],
) -> Vec<String> {
    let author_names = authors
        .iter()
        .map(|author| author.name.to_ascii_lowercase())
        .collect::<Vec<_>>();
    candidate
        .blocks
        .iter()
        .flat_map(|block| block.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| *line != title)
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("doi.org")
                || lower.contains("check for updates")
                || lower.contains("published")
                || lower.contains("received")
                || lower.contains("accepted")
                || looks_like_affiliation_line(line)
                || looks_like_contact_line(line)
                || looks_like_copyright_or_license_line(line)
                || author_names.iter().any(|name| lower.contains(name))
        })
        .map(ToString::to_string)
        .collect()
}

fn build_front_matter_translation_prompt(
    front_matter: &AcademicFrontMatter,
) -> Result<String, AppError> {
    let payload = serde_json::to_string_pretty(front_matter).map_err(|error| {
        AppError::internal(format!(
            "serialize front matter translation payload failed: {error}"
        ))
    })?;
    Ok(format!(
        "You are translating academic front-matter metadata from English into Simplified Chinese.\n\
         Return JSON only. Do not add Markdown or HTML.\n\
         Keep the same schema and preserve author order.\n\
         Translate the article title into natural Simplified Chinese.\n\
         Translate affiliations, author notes, equal-contribution notes, and non-DOI extras into Simplified Chinese.\n\
         Keep personal names, email addresses, DOI links, and explicit publication identifiers in the original language.\n\
         Keep markers unchanged.\n\
         Keep affiliation refs and extra kind labels structurally stable.\n\
         If an extra is only a DOI URL, keep it unchanged.\n\
         If an extra is an English status label like 'Check for updates', translate it.\n\
         JSON schema:\n\
         {{\"title\":\"...\",\"authors\":[{{\"name\":\"...\",\"email\":null,\"markers\":[\"1\"],\"affiliations\":[\"...\"],\"affiliationRefs\":[\"1\"],\"isCorresponding\":false,\"equalContribution\":[\"...\"],\"authorNotes\":[\"...\"],\"correspondenceNote\":null}}],\"extras\":[\"...\"],\"affiliationCatalog\":[{{\"id\":\"1\",\"text\":\"...\"}}],\"extraEntries\":[{{\"raw\":\"...\",\"kind\":\"doi|receivedDate|acceptedDate|publishedDate|journalMeta|articleNote|other\"}}]}}\n\n\
         Source JSON:\n{payload}"
    ))
}

fn build_front_matter_translation_repair_prompt(
    source: &AcademicFrontMatter,
    translated: &AcademicFrontMatter,
) -> Result<String, AppError> {
    let source_payload = serde_json::to_string_pretty(source).map_err(|error| {
        AppError::internal(format!(
            "serialize source front matter repair payload failed: {error}"
        ))
    })?;
    let translated_payload = serde_json::to_string_pretty(translated).map_err(|error| {
        AppError::internal(format!(
            "serialize translated front matter repair payload failed: {error}"
        ))
    })?;
    Ok(format!(
        "You are repairing an academic front-matter JSON translation.\n\
         Return JSON only. Keep exactly the same schema, author order, markers, emails, DOI links, and structural ids.\n\
         Use Source JSON only as reference. Repair only fields that should be Simplified Chinese but are still English.\n\
         Translate title, affiliations, affiliation catalog text, author notes, equal-contribution notes, correspondence notes, and non-DOI extras into natural Simplified Chinese.\n\
         Keep personal names, email addresses, DOI links, journal identifiers, and explicit publication identifiers unchanged.\n\
         Do not invent or delete authors, affiliations, or extras.\n\n\
         Source JSON:\n{source_payload}\n\n\
         Current translated JSON:\n{translated_payload}"
    ))
}

fn front_matter_needs_translation_repair(
    source: &AcademicFrontMatter,
    translated: &AcademicFrontMatter,
) -> bool {
    source
        .title
        .as_deref()
        .zip(translated.title.as_deref())
        .is_some_and(|(source_value, translated_value)| {
            should_translate_front_matter_value(source_value, translated_value)
        })
        || paired_author_values_need_translation_repair(source, translated)
        || paired_values_need_translation_repair(&source.extras, &translated.extras)
        || source
            .affiliation_catalog
            .iter()
            .zip(translated.affiliation_catalog.iter())
            .any(|(source_entry, translated_entry)| {
                should_translate_front_matter_value(&source_entry.text, &translated_entry.text)
            })
}

fn paired_author_values_need_translation_repair(
    source: &AcademicFrontMatter,
    translated: &AcademicFrontMatter,
) -> bool {
    source.authors.iter().zip(translated.authors.iter()).any(
        |(source_author, translated_author)| {
            paired_values_need_translation_repair_relaxed(
                &source_author.affiliations,
                &translated_author.affiliations,
            ) || paired_values_need_translation_repair(
                &source_author.author_notes,
                &translated_author.author_notes,
            ) || paired_values_need_translation_repair(
                &source_author.equal_contribution,
                &translated_author.equal_contribution,
            ) || source_author
                .correspondence_note
                .as_deref()
                .zip(translated_author.correspondence_note.as_deref())
                .is_some_and(|(source_value, translated_value)| {
                    should_translate_front_matter_value(source_value, translated_value)
                })
        },
    )
}

fn paired_values_need_translation_repair_relaxed(source: &[String], translated: &[String]) -> bool {
    source
        .iter()
        .zip(translated.iter())
        .any(|(source_value, translated_value)| {
            should_translate_front_matter_value_relaxed(source_value, translated_value)
        })
}

fn paired_values_need_translation_repair(source: &[String], translated: &[String]) -> bool {
    source
        .iter()
        .zip(translated.iter())
        .any(|(source_value, translated_value)| {
            should_translate_front_matter_value(source_value, translated_value)
        })
}

fn should_translate_front_matter_value(source: &str, translated: &str) -> bool {
    should_translate_front_matter_value_base(source, translated)
        && normalize_front_matter_repair_key(source)
            == normalize_front_matter_repair_key(translated)
}

fn should_translate_front_matter_value_relaxed(source: &str, translated: &str) -> bool {
    should_translate_front_matter_value_base(source, translated)
}

fn should_translate_front_matter_value_base(source: &str, translated: &str) -> bool {
    let source = source.trim();
    let translated = translated.trim();
    if source.is_empty()
        || translated.is_empty()
        || front_matter_value_should_remain_english(source)
    {
        return false;
    }
    let source_is_english = ascii_letter_ratio(source) >= 0.45;
    let translated_is_still_english = ascii_letter_ratio(translated) >= 0.45;
    source_is_english && translated_is_still_english && !has_cjk_text(translated)
}

fn front_matter_value_should_remain_english(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    lower.starts_with("doi:")
        || lower.contains("doi.org")
        || (lower.contains('@') && !has_cjk_text(trimmed))
}

fn ascii_letter_ratio(value: &str) -> f32 {
    let mut total = 0usize;
    let mut ascii_letters = 0usize;
    for ch in value.chars() {
        if ch.is_whitespace() || ch.is_ascii_punctuation() || ch.is_ascii_digit() {
            continue;
        }
        total += 1;
        if ch.is_ascii_alphabetic() {
            ascii_letters += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        ascii_letters as f32 / total as f32
    }
}

fn has_cjk_text(value: &str) -> bool {
    value
        .chars()
        .any(|ch| matches!(ch as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF))
}

fn normalize_front_matter_repair_key(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

pub(super) fn parse_front_matter_response(raw: &str) -> Result<FrontMatterExtraction, AppError> {
    let json = extract_json_object(raw)
        .ok_or_else(|| AppError::internal("front matter LLM response missing JSON object"))?;
    let parsed = serde_json::from_str::<FrontMatterExtraction>(json)
        .map_err(|error| AppError::internal(format!("front matter JSON parse failed: {error}")))?;
    sanitize_front_matter(parsed)
}

fn enrich_front_matter_from_candidate(
    mut front_matter: FrontMatterExtraction,
    candidate: &HeaderBlockCandidate,
) -> FrontMatterExtraction {
    if front_matter.authors.is_empty()
        || front_matter
            .authors
            .iter()
            .any(|author| !author.affiliations.is_empty() || !author.affiliation_refs.is_empty())
    {
        return front_matter;
    }

    let affiliations = candidate_affiliation_values(candidate);
    if affiliations.len() == front_matter.authors.len() {
        for (author, affiliation) in front_matter.authors.iter_mut().zip(affiliations) {
            author.affiliations.push(affiliation);
        }
    } else if affiliations.len() == 1 {
        for author in &mut front_matter.authors {
            author.affiliations.push(affiliations[0].clone());
        }
    }
    front_matter
}

fn candidate_affiliation_values(candidate: &HeaderBlockCandidate) -> Vec<String> {
    let mut affiliations = Vec::new();
    for line in candidate.blocks.iter().flat_map(|block| block.lines()) {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.ends_with(':')
            || looks_like_contact_line(trimmed)
            || !looks_like_affiliation_line(trimmed)
        {
            continue;
        }
        affiliations.extend(split_compound_affiliation_line(trimmed));
    }
    dedupe_preserve_order(affiliations)
}

fn split_compound_affiliation_line(line: &str) -> Vec<String> {
    let mut remaining = line.trim();
    let mut parts = Vec::new();
    while let Some(split_at) = next_affiliation_conjunction_split(remaining) {
        let (left, right) = remaining.split_at(split_at);
        let left = left.trim();
        if !left.is_empty() {
            parts.push(left.to_string());
        }
        remaining = right.strip_prefix(" and ").unwrap_or(right).trim();
    }
    if !remaining.is_empty() {
        parts.push(remaining.to_string());
    }
    parts
}

fn next_affiliation_conjunction_split(value: &str) -> Option<usize> {
    let mut cursor = 0usize;
    while let Some(relative) = value[cursor..].find(" and ") {
        let split_at = cursor + relative;
        let right = &value[split_at + " and ".len()..];
        if starts_new_affiliation_phrase(right) {
            return Some(split_at);
        }
        cursor = split_at + " and ".len();
    }
    None
}

fn starts_new_affiliation_phrase(value: &str) -> bool {
    let lower = value.trim_start().to_ascii_lowercase();
    [
        "department",
        "school",
        "institute",
        "university",
        "college",
        "faculty",
        "centre",
        "center",
        "laboratory",
        "hospital",
        "cnrs",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

fn dedupe_preserve_order(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(value.to_ascii_lowercase()))
        .collect()
}

fn parse_translated_front_matter_response(raw: &str) -> Result<AcademicFrontMatter, AppError> {
    let json = extract_json_object(raw).ok_or_else(|| {
        AppError::internal("translated front matter response missing JSON object")
    })?;
    let parsed = serde_json::from_str::<AcademicFrontMatter>(json).map_err(|error| {
        AppError::internal(format!(
            "translated front matter JSON parse failed: {error}"
        ))
    })?;
    sanitize_academic_front_matter(parsed)
}

pub(super) fn front_matter_to_academic_front_matter(
    front_matter: FrontMatterExtraction,
) -> AcademicFrontMatter {
    let title = Some(front_matter.title);
    let extras = front_matter.extras;
    let affiliation_catalog = affiliation_catalog_map(&front_matter.affiliation_catalog);
    let normalized_authors = normalize_unmarked_repeated_affiliations(front_matter.authors);
    let authors = normalized_authors
        .into_iter()
        .filter(|author| !author.name.trim().is_empty())
        .map(|author| front_matter_author_to_academic_author(author, &affiliation_catalog))
        .collect::<Vec<_>>();
    AcademicFrontMatter {
        title,
        affiliation_catalog: build_affiliation_catalog(&authors),
        extra_entries: classify_front_matter_extras(&extras),
        authors,
        extras,
    }
}

fn affiliation_catalog_map(
    entries: &[FrontMatterAffiliationEntry],
) -> std::collections::BTreeMap<String, String> {
    entries
        .iter()
        .filter_map(|entry| {
            let id = entry.id.trim();
            let text = entry.text.trim();
            (!id.is_empty() && !text.is_empty()).then(|| (id.to_string(), text.to_string()))
        })
        .collect()
}

fn normalize_unmarked_repeated_affiliations(
    authors: Vec<FrontMatterAuthor>,
) -> Vec<FrontMatterAuthor> {
    if authors.len() < 2
        || authors
            .iter()
            .any(|author| !trim_nonempty_values(author.affiliation_refs.clone()).is_empty())
    {
        return authors;
    }

    let Some(shared_affiliations) = repeated_affiliation_list(&authors) else {
        return authors;
    };
    if shared_affiliations.len() != authors.len() {
        return authors;
    }

    authors
        .into_iter()
        .enumerate()
        .map(|(index, mut author)| {
            author.affiliations = shared_affiliations
                .get(index)
                .cloned()
                .into_iter()
                .collect::<Vec<_>>();
            author
        })
        .collect()
}

fn repeated_affiliation_list(authors: &[FrontMatterAuthor]) -> Option<Vec<String>> {
    let first = trim_nonempty_values(authors.first()?.affiliations.clone());
    if first.len() < 2 {
        return None;
    }
    authors
        .iter()
        .all(|author| trim_nonempty_values(author.affiliations.clone()) == first)
        .then_some(first)
}

pub(super) fn rewrite_source_markdown_with_front_matter(
    markdown: &str,
    front_matter: &AcademicFrontMatter,
) -> String {
    if !crate::pipeline::render::authors::has_academic_front_matter(front_matter) {
        return markdown.to_string();
    }

    let body = crate::pipeline::render::strip_front_matter_region(markdown, front_matter);
    let rebuilt_header = render_source_front_matter_markdown(front_matter);

    if body.trim().is_empty() {
        return rebuilt_header;
    }
    format!("{rebuilt_header}\n\n{body}")
}

fn render_source_front_matter_markdown(front_matter: &AcademicFrontMatter) -> String {
    let mut blocks = Vec::new();

    if let Some(title) = front_matter
        .title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
    {
        blocks.push(format!("# {title}"));
    }

    if !front_matter.authors.is_empty() {
        blocks.push(render_source_author_line(&front_matter.authors));
        let detail_lines = render_source_author_detail_lines(&front_matter.authors);
        if !detail_lines.is_empty() {
            blocks.push(detail_lines.join("\n"));
        }
    }

    let extras = front_matter
        .extras
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .filter(|item| !is_update_notice_block(item))
        .collect::<Vec<_>>();
    if !extras.is_empty() {
        blocks.push(extras.join("\n"));
    }

    format!(
        "{}\n\n{}\n\n{}",
        crate::pipeline::chunking::FRONT_MATTER_PROTECTED_START,
        blocks.join("\n\n"),
        crate::pipeline::chunking::FRONT_MATTER_PROTECTED_END
    )
}

fn render_source_author_line(authors: &[AcademicAuthor]) -> String {
    authors
        .iter()
        .map(render_source_author_inline)
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_source_author_inline(author: &AcademicAuthor) -> String {
    let markers = author
        .markers
        .iter()
        .map(|marker| marker.trim())
        .filter(|marker| !marker.is_empty())
        .collect::<Vec<_>>();
    if markers.is_empty() {
        return author.name.trim().to_string();
    }
    format!("{} $ ^{{{}}} $", author.name.trim(), markers.join(","))
}

fn render_source_author_detail_lines(authors: &[AcademicAuthor]) -> Vec<String> {
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;

    let mut affiliation_map: BTreeMap<String, String> = BTreeMap::new();
    let mut appended_plain_affiliations = BTreeSet::new();
    let mut trailing_notes = Vec::new();

    for author in authors {
        for (index, affiliation) in author.affiliations.iter().enumerate() {
            let trimmed = affiliation.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(marker) = author.markers.get(index).map(|item| item.trim()) {
                if is_affiliation_ref_marker(marker) {
                    affiliation_map
                        .entry(marker.to_string())
                        .or_insert_with(|| trimmed.to_string());
                    continue;
                }
            }
            if appended_plain_affiliations.insert(trimmed.to_string()) {
                trailing_notes.push(trimmed.to_string());
            }
        }
        if author.is_corresponding {
            if let Some(note) = author.correspondence_note.as_deref().map(str::trim) {
                if !note.is_empty() {
                    trailing_notes.push(format!("* {note}"));
                }
            }
            if let Some(email) = author.email.as_deref().map(str::trim) {
                if !email.is_empty() {
                    trailing_notes.push(format!("Email: {email}"));
                }
            }
        }
    }

    let mut lines = affiliation_map
        .into_iter()
        .map(|(marker, value)| format!("{marker} {value}"))
        .collect::<Vec<_>>();
    lines.extend(trailing_notes);
    lines
}

fn front_matter_author_to_academic_author(
    author: FrontMatterAuthor,
    affiliation_catalog: &std::collections::BTreeMap<String, String>,
) -> AcademicAuthor {
    let normalized_affiliation_refs = trim_nonempty_values(author.affiliation_refs);
    let normalized_affiliations = resolve_author_affiliations(
        author.affiliations,
        &normalized_affiliation_refs,
        affiliation_catalog,
    );
    let normalized_equal_contribution = trim_nonempty_values(author.equal_contribution);
    let normalized_author_notes = trim_nonempty_values(author.author_notes);
    let normalized_notes = trim_nonempty_values(author.notes);
    let fallback_notes = if normalized_author_notes.is_empty() {
        normalized_notes.clone()
    } else {
        normalized_author_notes.clone()
    };
    AcademicAuthor {
        name: author.name,
        email: author.email,
        markers: author.markers,
        affiliations: normalized_affiliations,
        is_corresponding: author.is_corresponding,
        affiliation_refs: normalized_affiliation_refs,
        equal_contribution: if normalized_equal_contribution.is_empty() {
            infer_equal_contribution_notes(&normalized_notes)
        } else {
            normalized_equal_contribution
        },
        author_notes: fallback_notes.clone(),
        correspondence_note: normalized_notes.into_iter().next(),
    }
}

fn resolve_author_affiliations(
    affiliations: Vec<String>,
    affiliation_refs: &[String],
    affiliation_catalog: &std::collections::BTreeMap<String, String>,
) -> Vec<String> {
    let direct = trim_nonempty_values(affiliations);
    if !direct.is_empty() {
        return direct;
    }
    affiliation_refs
        .iter()
        .filter_map(|reference| affiliation_catalog.get(reference.trim()).cloned())
        .collect()
}

fn sanitize_front_matter(raw: FrontMatterExtraction) -> Result<FrontMatterExtraction, AppError> {
    let title = raw.title.trim().to_string();
    if title.is_empty() {
        return Err(AppError::internal("front matter title is empty"));
    }
    let authors = raw
        .authors
        .into_iter()
        .filter_map(sanitize_front_matter_author)
        .collect::<Vec<_>>();
    let extras = raw
        .extras
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .filter(|item| !should_drop_front_matter_extra(item))
        .collect();
    Ok(FrontMatterExtraction {
        title,
        authors,
        extras,
        affiliation_catalog: sanitize_front_matter_affiliation_catalog(raw.affiliation_catalog),
        extra_entries: sanitize_front_matter_extra_entries(raw.extra_entries),
    })
}

fn sanitize_front_matter_author(author: FrontMatterAuthor) -> Option<FrontMatterAuthor> {
    let name = author.name.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(FrontMatterAuthor {
        name,
        markers: trim_nonempty_values(author.markers),
        affiliations: trim_nonempty_values(author.affiliations),
        affiliation_refs: trim_nonempty_values(author.affiliation_refs),
        email: author
            .email
            .map(|email| email.trim().to_string())
            .filter(|email| !email.is_empty()),
        is_corresponding: author.is_corresponding,
        equal_contribution: trim_nonempty_values(author.equal_contribution),
        author_notes: trim_nonempty_values(author.author_notes),
        notes: trim_nonempty_values(author.notes),
    })
}

fn sanitize_academic_front_matter(
    raw: AcademicFrontMatter,
) -> Result<AcademicFrontMatter, AppError> {
    let title = raw
        .title
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty());
    if title.is_none() {
        return Err(AppError::internal("translated front matter title is empty"));
    }
    let authors = raw
        .authors
        .into_iter()
        .filter_map(|author| {
            let name = author.name.trim().to_string();
            if name.is_empty() {
                return None;
            }
            Some(AcademicAuthor {
                name,
                email: author
                    .email
                    .map(|email| email.trim().to_string())
                    .filter(|email| !email.is_empty()),
                markers: trim_nonempty_values(author.markers),
                affiliations: trim_nonempty_values(author.affiliations),
                is_corresponding: author.is_corresponding,
                affiliation_refs: trim_nonempty_values(author.affiliation_refs),
                equal_contribution: trim_nonempty_values(author.equal_contribution),
                author_notes: trim_nonempty_values(author.author_notes),
                correspondence_note: author
                    .correspondence_note
                    .map(|note| note.trim().to_string())
                    .filter(|note| !note.is_empty()),
            })
        })
        .collect::<Vec<_>>();
    let extras = trim_nonempty_values(raw.extras)
        .into_iter()
        .filter(|item| !should_drop_front_matter_extra(item))
        .collect::<Vec<_>>();
    let affiliation_catalog = build_affiliation_catalog(&authors);
    let extra_entries = classify_front_matter_extras(&extras);
    Ok(AcademicFrontMatter {
        title,
        authors,
        extras,
        affiliation_catalog,
        extra_entries,
    })
}

fn build_affiliation_catalog(authors: &[AcademicAuthor]) -> Vec<AcademicAffiliationEntry> {
    let mut entries = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for author in authors {
        for (index, affiliation) in author.affiliations.iter().enumerate() {
            let text = affiliation.trim();
            if text.is_empty() {
                continue;
            }
            let id = author
                .affiliation_refs
                .get(index)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .or_else(|| {
                    author
                        .markers
                        .get(index)
                        .map(|value| value.trim().to_string())
                        .filter(|value| is_affiliation_ref_marker(value))
                })
                .unwrap_or_else(|| text.to_string());
            let key = format!("{id}|{text}");
            if seen.insert(key) {
                entries.push(AcademicAffiliationEntry {
                    id,
                    text: text.to_string(),
                });
            }
        }
    }
    entries
}

fn is_affiliation_ref_marker(marker: &str) -> bool {
    let trimmed = marker.trim();
    !trimmed.is_empty()
        && trimmed != "*"
        && !trimmed.eq_ignore_ascii_case("id")
        && trimmed.chars().any(|ch| ch.is_alphanumeric())
}

fn classify_front_matter_extras(extras: &[String]) -> Vec<AcademicFrontMatterExtras> {
    extras
        .iter()
        .map(|raw| AcademicFrontMatterExtras {
            raw: raw.clone(),
            kind: classify_front_matter_extra_kind(raw).to_string(),
        })
        .collect()
}

fn build_front_matter_affiliation_catalog(
    authors: &[AcademicAuthor],
) -> Vec<FrontMatterAffiliationEntry> {
    build_affiliation_catalog(authors)
        .into_iter()
        .map(|entry| FrontMatterAffiliationEntry {
            id: entry.id,
            text: entry.text,
        })
        .collect()
}

fn classify_front_matter_extra_entries(extras: &[String]) -> Vec<FrontMatterExtraEntry> {
    classify_front_matter_extras(extras)
        .into_iter()
        .map(|entry| FrontMatterExtraEntry {
            raw: entry.raw,
            kind: entry.kind,
        })
        .collect()
}

fn sanitize_front_matter_affiliation_catalog(
    entries: Vec<FrontMatterAffiliationEntry>,
) -> Vec<FrontMatterAffiliationEntry> {
    let mut seen = std::collections::BTreeSet::new();
    entries
        .into_iter()
        .filter_map(|entry| {
            let id = entry.id.trim().to_string();
            let text = entry.text.trim().to_string();
            if id.is_empty() || text.is_empty() {
                return None;
            }
            let key = format!("{id}|{text}");
            seen.insert(key)
                .then_some(FrontMatterAffiliationEntry { id, text })
        })
        .collect()
}

fn sanitize_front_matter_extra_entries(
    entries: Vec<FrontMatterExtraEntry>,
) -> Vec<FrontMatterExtraEntry> {
    let mut seen = std::collections::BTreeSet::new();
    entries
        .into_iter()
        .filter_map(|entry| {
            let raw = entry.raw.trim().to_string();
            let kind = entry.kind.trim().to_string();
            if raw.is_empty() || kind.is_empty() || should_drop_front_matter_extra(&raw) {
                return None;
            }
            let key = format!("{raw}|{kind}");
            seen.insert(key)
                .then_some(FrontMatterExtraEntry { raw, kind })
        })
        .collect()
}

fn should_drop_front_matter_extra(item: &str) -> bool {
    let lower = item.to_ascii_lowercase();
    looks_like_copyright_or_license_line(item)
        && (lower.contains("creative commons")
            || lower.contains("open access")
            || lower.contains("the author(s)")
            || lower.contains("the authors"))
}

fn classify_front_matter_extra_kind(raw: &str) -> &'static str {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.contains("doi.org") {
        "doi"
    } else if normalized.contains("received") {
        "receivedDate"
    } else if normalized.contains("accepted") {
        "acceptedDate"
    } else if normalized.contains("published") || normalized.contains("online") {
        "publishedDate"
    } else if normalized.contains("journal") || normalized.contains("nature") {
        "journalMeta"
    } else if normalized.contains("update") {
        "articleNote"
    } else {
        "other"
    }
}

fn infer_equal_contribution_notes(notes: &[String]) -> Vec<String> {
    notes
        .iter()
        .filter(|note| {
            let lower = note.to_ascii_lowercase();
            lower.contains("contributed equally") || lower.contains("equal contribution")
        })
        .cloned()
        .collect()
}

fn trim_nonempty_values(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn extract_json_object(input: &str) -> Option<&str> {
    let trimmed = input.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed);
    }
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')?;
    (start < end).then_some(&trimmed[start..=end])
}

fn split_blocks(markdown: &str) -> Vec<String> {
    markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn detect_header_boundary_index(blocks: &[String]) -> (usize, HeaderBoundary) {
    if blocks.is_empty() {
        return (0, HeaderBoundary::DocumentEnd);
    }

    let first_section_heading = blocks
        .iter()
        .position(|block| classify_header_block(block) == HeaderBlockKind::SectionHeading);

    let mut seen_title = false;
    let mut seen_authorish = false;
    let mut seen_front_matter = false;
    let mut candidate_body_index = None;

    for (index, block) in blocks.iter().enumerate() {
        let kind = classify_header_block(block);
        match kind {
            HeaderBlockKind::Title => {
                seen_title = true;
                seen_front_matter = true;
                continue;
            }
            HeaderBlockKind::AuthorLine
            | HeaderBlockKind::AffiliationLine
            | HeaderBlockKind::ContactLine
            | HeaderBlockKind::DoiOrJournalMeta
            | HeaderBlockKind::UpdateNotice
            | HeaderBlockKind::CopyrightOrLicense
            | HeaderBlockKind::UnknownFrontMatter => {
                seen_authorish = seen_authorish
                    || matches!(
                        kind,
                        HeaderBlockKind::AuthorLine
                            | HeaderBlockKind::AffiliationLine
                            | HeaderBlockKind::ContactLine
                    );
                seen_front_matter = true;
                continue;
            }
            HeaderBlockKind::AbstractHeading => {
                return (index, HeaderBoundary::FirstSecondLevelHeading);
            }
            HeaderBlockKind::SectionHeading => {
                return (index, HeaderBoundary::FirstSecondLevelHeading);
            }
            HeaderBlockKind::BodyParagraph => {
                if is_stable_body_start(
                    blocks,
                    index,
                    seen_title,
                    seen_authorish,
                    seen_front_matter,
                ) {
                    return (index, HeaderBoundary::FirstBodyParagraph);
                }
                candidate_body_index.get_or_insert(index);
            }
            HeaderBlockKind::Empty => {}
        }
    }

    if let Some(index) = candidate_body_index {
        return (index, HeaderBoundary::FirstBodyParagraph);
    }
    if let Some(index) = first_section_heading {
        return (index, HeaderBoundary::FirstSecondLevelHeading);
    }
    (blocks.len(), HeaderBoundary::DocumentEnd)
}

fn classify_header_block(block: &str) -> HeaderBlockKind {
    let trimmed = block.trim();
    if trimmed.is_empty() {
        return HeaderBlockKind::Empty;
    }
    if trimmed.starts_with("## ") {
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("## abstract") {
            return HeaderBlockKind::AbstractHeading;
        }
        return HeaderBlockKind::SectionHeading;
    }
    if trimmed.starts_with('#') {
        return HeaderBlockKind::Title;
    }
    if is_update_notice_block(trimmed) {
        return HeaderBlockKind::UpdateNotice;
    }
    if looks_like_doi_or_journal_meta_line(trimmed) {
        return HeaderBlockKind::DoiOrJournalMeta;
    }
    if looks_like_author_line(trimmed) {
        return HeaderBlockKind::AuthorLine;
    }
    if looks_like_contact_line(trimmed) {
        return HeaderBlockKind::ContactLine;
    }
    if looks_like_affiliation_line(trimmed) {
        return HeaderBlockKind::AffiliationLine;
    }
    if looks_like_copyright_or_license_line(trimmed) {
        return HeaderBlockKind::CopyrightOrLicense;
    }
    if looks_like_unknown_front_matter_line(trimmed) {
        return HeaderBlockKind::UnknownFrontMatter;
    }
    if looks_like_body_paragraph(trimmed) {
        return HeaderBlockKind::BodyParagraph;
    }
    HeaderBlockKind::UnknownFrontMatter
}

fn looks_like_body_paragraph(block: &str) -> bool {
    if is_non_body_header_block(block) {
        return false;
    }
    let chars = block.chars().count();
    chars >= 180 && has_sentence_punctuation(block) && has_enough_words(block)
}

fn is_non_body_header_block(block: &str) -> bool {
    matches!(
        classify_header_block_without_body(block),
        HeaderBlockKind::Title
            | HeaderBlockKind::AbstractHeading
            | HeaderBlockKind::SectionHeading
            | HeaderBlockKind::AuthorLine
            | HeaderBlockKind::AffiliationLine
            | HeaderBlockKind::ContactLine
            | HeaderBlockKind::DoiOrJournalMeta
            | HeaderBlockKind::UpdateNotice
            | HeaderBlockKind::CopyrightOrLicense
            | HeaderBlockKind::UnknownFrontMatter
    )
}

fn classify_header_block_without_body(block: &str) -> HeaderBlockKind {
    let trimmed = block.trim();
    if trimmed.is_empty() {
        return HeaderBlockKind::Empty;
    }
    if trimmed.starts_with("## ") {
        let lower = trimmed.to_ascii_lowercase();
        if lower.starts_with("## abstract") {
            return HeaderBlockKind::AbstractHeading;
        }
        return HeaderBlockKind::SectionHeading;
    }
    if trimmed.starts_with('#') {
        return HeaderBlockKind::Title;
    }
    if trimmed.starts_with("http://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("doi:")
        || trimmed.starts_with("![](")
        || trimmed.starts_with("<!--")
        || trimmed.starts_with('|')
        || is_update_notice_block(trimmed)
    {
        return HeaderBlockKind::DoiOrJournalMeta;
    }
    if looks_like_author_line(trimmed) {
        return HeaderBlockKind::AuthorLine;
    }
    if looks_like_contact_line(trimmed) {
        return HeaderBlockKind::ContactLine;
    }
    if looks_like_affiliation_line(trimmed) {
        return HeaderBlockKind::AffiliationLine;
    }
    if looks_like_copyright_or_license_line(trimmed) {
        return HeaderBlockKind::CopyrightOrLicense;
    }
    if looks_like_unknown_front_matter_line(trimmed) {
        return HeaderBlockKind::UnknownFrontMatter;
    }
    HeaderBlockKind::Empty
}

fn is_update_notice_block(block: &str) -> bool {
    let lower = block.to_ascii_lowercase();
    lower.contains("check for updates")
        || lower.contains("check for update")
        || lower.contains("view pdf")
}

fn looks_like_author_line(block: &str) -> bool {
    let lower = block.to_ascii_lowercase();
    ((block.contains(" $ ^{") || block.contains(r"\textcircled") || block.contains('&'))
        && !lower.starts_with("abstract")
        && block.chars().count() < 320)
        || (contains_multiple_person_like_names(block) && block.chars().count() < 360)
}

fn looks_like_affiliation_line(block: &str) -> bool {
    let lower = block.to_ascii_lowercase();
    (lower.contains("department")
        || lower.contains("university")
        || lower.contains("institute")
        || lower.contains("school")
        || lower.contains("laboratory")
        || lower.contains("centre")
        || lower.contains("center")
        || lower.contains("college")
        || lower.contains("cnrs")
        || lower.contains("faculty")
        || lower.contains("hospital"))
        && !lower.starts_with("abstract")
}

fn looks_like_contact_line(block: &str) -> bool {
    let lower = block.to_ascii_lowercase();
    block.contains('@')
        || lower.contains("e-mail")
        || lower.contains("email:")
        || lower.contains("corresponding author")
        || lower.contains("correspondence")
        || lower.contains("these authors contributed equally")
}

fn looks_like_doi_or_journal_meta_line(block: &str) -> bool {
    let lower = block.to_ascii_lowercase();
    lower.contains("doi.org")
        || lower.starts_with("doi:")
        || lower.contains("published online")
        || lower.contains("received:")
        || lower.contains("accepted:")
        || lower.contains("nature ")
        || lower.contains("journal")
}

fn looks_like_copyright_or_license_line(block: &str) -> bool {
    let lower = block.to_ascii_lowercase();
    lower.contains("copyright")
        || lower.contains("license")
        || lower.contains("creativecommons")
        || lower.contains("open access")
        || lower.contains("publisher")
}

fn looks_like_unknown_front_matter_line(block: &str) -> bool {
    let lower = block.to_ascii_lowercase();
    lower.starts_with("first-page footnote:")
        || lower.starts_with("article")
        || lower.starts_with("review")
        || lower.starts_with("preprint")
        || lower.starts_with("supplementary")
}

fn contains_multiple_person_like_names(block: &str) -> bool {
    let uppercase_tokens = block
        .split_whitespace()
        .filter(|token| {
            let cleaned = token.trim_matches(|ch: char| !ch.is_alphabetic());
            cleaned.len() > 1
                && cleaned
                    .chars()
                    .next()
                    .map(|ch| ch.is_ascii_uppercase())
                    .unwrap_or(false)
        })
        .count();
    uppercase_tokens >= 4
}

fn is_stable_body_start(
    blocks: &[String],
    index: usize,
    seen_title: bool,
    seen_authorish: bool,
    seen_front_matter: bool,
) -> bool {
    let current = blocks
        .get(index)
        .map(|block| classify_header_block(block))
        .unwrap_or(HeaderBlockKind::Empty);
    if current != HeaderBlockKind::BodyParagraph {
        return false;
    }
    let next_is_body = blocks
        .get(index + 1)
        .map(|block| classify_header_block(block) == HeaderBlockKind::BodyParagraph)
        .unwrap_or(false);
    let next_is_section = blocks
        .get(index + 1)
        .map(|block| classify_header_block(block) == HeaderBlockKind::SectionHeading)
        .unwrap_or(false);
    (seen_title && seen_front_matter && (seen_authorish || next_is_body || next_is_section))
        || (seen_title && next_is_body)
}

fn header_candidate_needs_boundary_assist(candidate: &HeaderBlockCandidate) -> bool {
    if candidate.blocks.is_empty() {
        return false;
    }
    let likely_affiliation_count = candidate
        .blocks
        .iter()
        .filter(|block| looks_like_affiliation_line(block))
        .count();
    let likely_author_count = candidate
        .blocks
        .iter()
        .filter(|block| looks_like_author_line(block))
        .count();
    candidate.body_start_block <= 2
        || (likely_author_count > 0 && likely_affiliation_count == 0)
        || candidate.blocks.len() <= 2
}

async fn refine_header_candidate_with_llm(
    llm_client: &SensenovaClient,
    candidate: &HeaderBlockCandidate,
) -> Result<HeaderBlockCandidate, AppError> {
    let prompt = build_header_boundary_assist_prompt(candidate);
    let response = llm_client
        .review_translation_issue(&prompt)
        .await
        .map_err(AppError::from_provider_error)?;
    let decision = parse_header_boundary_assist_response(&response)?;
    Ok(apply_header_boundary_assist(candidate, decision))
}

fn build_header_boundary_assist_prompt(candidate: &HeaderBlockCandidate) -> String {
    let header_blocks = candidate
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| format!("BLOCK {index}:\n{block}"))
        .collect::<Vec<_>>()
        .join("\n\n");
    let probe_blocks = candidate
        .probe_blocks
        .iter()
        .enumerate()
        .map(|(offset, block)| {
            format!(
                "PROBE BLOCK {}:\n{}",
                candidate.body_start_block + offset,
                block
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    format!(
        "You are assisting boundary detection for academic PDF front matter.\n\
         Return JSON only.\n\
         Decide whether the current header candidate is missing affiliation/contact/metadata blocks that still belong to the article front matter.\n\
         If the current candidate is already sufficient, return keepCurrent=true and bodyStartBlock as-is.\n\
         If some listed blocks still belong to front matter, extend bodyStartBlock to include them.\n\
         Never move bodyStartBlock earlier than the current candidate length.\n\
         JSON schema:\n\
         {{\"keepCurrent\":true,\"bodyStartBlock\":3,\"reason\":\"...\"}}\n\n\
         Current bodyStartBlock: {}\n\
         Candidate header blocks:\n{}\n\n\
         Immediate probe blocks after the current boundary:\n{}",
        candidate.body_start_block, header_blocks, probe_blocks
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeaderBoundaryAssistDecision {
    keep_current: bool,
    body_start_block: usize,
    #[allow(dead_code)]
    reason: Option<String>,
}

fn parse_header_boundary_assist_response(
    raw: &str,
) -> Result<HeaderBoundaryAssistDecision, AppError> {
    let json = extract_json_object(raw)
        .ok_or_else(|| AppError::internal("header-boundary assist response missing JSON object"))?;
    serde_json::from_str::<HeaderBoundaryAssistDecision>(json).map_err(|error| {
        AppError::internal(format!("header-boundary assist JSON parse failed: {error}"))
    })
}

fn apply_header_boundary_assist(
    candidate: &HeaderBlockCandidate,
    decision: HeaderBoundaryAssistDecision,
) -> HeaderBlockCandidate {
    if decision.keep_current || decision.body_start_block <= candidate.body_start_block {
        return candidate.clone();
    }
    let extension_count = decision
        .body_start_block
        .saturating_sub(candidate.body_start_block)
        .min(candidate.probe_blocks.len());
    let mut blocks = candidate.blocks.clone();
    blocks.extend(candidate.probe_blocks.iter().take(extension_count).cloned());
    HeaderBlockCandidate {
        blocks,
        probe_blocks: candidate
            .probe_blocks
            .iter()
            .skip(extension_count)
            .cloned()
            .collect(),
        body_start_block: decision.body_start_block,
        boundary: candidate.boundary,
    }
}

fn has_sentence_punctuation(block: &str) -> bool {
    block.contains('.')
        || block.contains('?')
        || block.contains('!')
        || block.contains('。')
        || block.contains('？')
        || block.contains('！')
}

fn has_enough_words(block: &str) -> bool {
    block.split_whitespace().take(35).count() >= 35
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_empirical_pdf_header_before_abstract_like_body() {
        let markdown = concat!(
            "https://doi.org/10.1038/s44271-024-00170-w\n\n",
            "# Metacognition biases information seeking in assessing ambiguous news\n\n",
            "Check for updates\n\n",
            "Valentin Guigon $ ^{1,2} $, Marie Claire Villeval  $ \\textcircled{D}^{2,3,4} $ & Jean-Claude Dreher  $ \\textcircled{D}^{1,4} $\n\n",
            "How do we assess the veracity of ambiguous news, and does metacognition guide our decisions to seek further information? ",
            "In a controlled experiment, participants evaluated the veracity of ambiguous news and decided whether to seek extra information. ",
            "Confidence in their veracity judgments did not predict accuracy, showing limited metacognitive ability when facing ambiguous news. ",
            "Despite this, confidence in one's judgment was the primary driver of the demand for additional information about the news.\n\n",
            "The unprecedented growth of the internet and social media platforms has been accompanied by misinformation.\n\n",
            "## Methods Participants\n\n",
            "269 participants joined."
        );

        let candidate = detect_header_blocks(markdown);

        assert_eq!(candidate.boundary, HeaderBoundary::FirstBodyParagraph);
        assert_eq!(candidate.body_start_block, 4);
        assert_eq!(candidate.blocks.len(), 3);
        assert!(candidate.blocks[1].contains("Metacognition biases"));
        assert!(candidate.blocks[2].contains("Valentin Guigon"));
        assert!(!candidate
            .blocks
            .iter()
            .any(|block| block.starts_with("How do we assess")));
    }

    #[test]
    fn falls_back_to_first_second_level_heading_without_long_body() {
        let markdown = "https://example.test\n\n# Title\n\nAuthor One\n\n## Methods\n\nBody";

        let candidate = detect_header_blocks(markdown);

        assert_eq!(candidate.boundary, HeaderBoundary::FirstSecondLevelHeading);
        assert_eq!(candidate.body_start_block, 3);
        assert_eq!(candidate.blocks[0], "https://example.test");
        assert_eq!(candidate.blocks[1], "# Title");
        assert_eq!(candidate.blocks[2], "Author One");
        assert_eq!(candidate.blocks.len(), 3);
    }

    #[test]
    fn stops_front_matter_before_abstract_heading() {
        let markdown = concat!(
            "# Article Title\n\n",
            "Jane Doe $ ^{1} $\n\n",
            "1 Department A\n\n",
            "## Abstract\n\n",
            "This study examines a policy intervention.\n\n",
            "## Introduction\n\n",
            "Body."
        );

        let candidate = detect_header_blocks(markdown);

        assert_eq!(candidate.boundary, HeaderBoundary::FirstSecondLevelHeading);
        assert_eq!(candidate.body_start_block, 3);
        assert_eq!(candidate.blocks.len(), 3);
        assert!(!candidate
            .blocks
            .iter()
            .any(|block| block.contains("Abstract")));
    }

    #[test]
    fn keeps_short_titles_out_of_body_detection() {
        let markdown = "# A short title\n\nA brief subtitle.\n\n";

        let candidate = detect_header_blocks(markdown);

        assert_eq!(candidate.boundary, HeaderBoundary::DocumentEnd);
        assert_eq!(candidate.blocks.len(), 2);
    }

    #[test]
    fn keeps_affiliations_inside_header_window_before_abstract_body() {
        let markdown = concat!(
            "# Preprint Toolbox 2.0\n\n",
            "Alice Smith, Bob Jones, Carol White\n\n",
            "Department of Computer Science, Example University, Example City, Country.\n\n",
            "Institute for Reproducible Research, Another University, Another City, Country.\n\n",
            "This preprint introduces a modular toolbox for better scientific workflows. ",
            "It describes the motivation, implementation, evaluation, and ecosystem fit in enough detail ",
            "to look like an ordinary body paragraph for naive heuristics, but it still belongs under Abstract.\n\n",
            "## Introduction\n\n",
            "Main body."
        );

        let candidate = detect_header_blocks(markdown);

        assert_eq!(candidate.boundary, HeaderBoundary::FirstBodyParagraph);
        assert_eq!(candidate.body_start_block, 4);
        assert_eq!(candidate.blocks.len(), 4);
        assert!(candidate.blocks[2].contains("Department of Computer Science"));
        assert!(candidate.blocks[3].contains("Institute for Reproducible Research"));
    }

    #[test]
    fn flags_candidate_with_authors_but_without_affiliations_for_boundary_assist() {
        let candidate = HeaderBlockCandidate {
            blocks: vec![
                "# Example Article".to_string(),
                "Alice Smith $ ^{1} $, Bob Jones $ ^{2} $".to_string(),
            ],
            probe_blocks: vec![],
            body_start_block: 2,
            boundary: HeaderBoundary::FirstBodyParagraph,
        };

        assert!(header_candidate_needs_boundary_assist(&candidate));
    }

    #[test]
    fn builds_front_matter_prompt_with_strict_json_contract() {
        let candidate = HeaderBlockCandidate {
            blocks: vec![
                "https://doi.org/10.1038/example".to_string(),
                "# Article Title".to_string(),
                "Jane Doe $ ^{1} $ & John Roe $ ^{2} $".to_string(),
            ],
            probe_blocks: vec![],
            body_start_block: 3,
            boundary: HeaderBoundary::FirstBodyParagraph,
        };

        let prompt = build_front_matter_prompt(&candidate);

        assert!(prompt.contains("Return JSON only"));
        assert!(prompt.contains("\"title\""));
        assert!(prompt.contains("\"authors\""));
        assert!(prompt.contains("\"extras\""));
        assert!(prompt.contains("\"affiliationRefs\""));
        assert!(prompt.contains("\"equalContribution\""));
        assert!(prompt.contains("\"authorNotes\""));
        assert!(prompt.contains("\"affiliationCatalog\""));
        assert!(prompt.contains("\"extraEntries\""));
        assert!(prompt.contains("https://doi.org/10.1038/example"));
    }

    #[test]
    fn appends_footnote_context_without_shifting_header_boundary() {
        let candidate = HeaderBlockCandidate {
            blocks: vec![
                "# Article Title".to_string(),
                "Jane Doe $ ^{1} $".to_string(),
            ],
            probe_blocks: vec![],
            body_start_block: 2,
            boundary: HeaderBoundary::FirstBodyParagraph,
        };
        let footnotes = vec![
            "1 Department of Example, Example University.".to_string(),
            "e-mail: jane@example.test".to_string(),
        ];

        let expanded = append_candidate_context(&candidate, "First-page footnote", &footnotes);

        assert_eq!(expanded.body_start_block, 2);
        assert_eq!(expanded.boundary, HeaderBoundary::FirstBodyParagraph);
        assert_eq!(expanded.blocks.len(), 3);
        assert!(expanded.blocks[2].contains("First-page footnote"));
        assert!(expanded.blocks[2].contains("jane@example.test"));
    }

    #[test]
    fn extracts_first_page_footnote_blocks_from_layout() {
        let page_layouts = vec![OcrPageLayout {
            blocks: vec![
                crate::ocr::OcrLayoutBlock {
                    label: "text".to_string(),
                    content: "Body".to_string(),
                    bbox: vec![],
                    order: Some(2),
                    page: Some(0),
                },
                crate::ocr::OcrLayoutBlock {
                    label: "footnote".to_string(),
                    content: "2 Department B.".to_string(),
                    bbox: vec![],
                    order: Some(4),
                    page: Some(0),
                },
                crate::ocr::OcrLayoutBlock {
                    label: "footnote".to_string(),
                    content: "1 Department A.".to_string(),
                    bbox: vec![],
                    order: Some(3),
                    page: Some(0),
                },
            ],
        }];

        let footnotes = first_page_footnote_blocks(&page_layouts);

        assert_eq!(footnotes, vec!["1 Department A.", "2 Department B."]);
    }

    #[test]
    fn extracts_first_page_header_layout_before_abstract() {
        let page_layouts = vec![OcrPageLayout {
            blocks: vec![
                crate::ocr::OcrLayoutBlock {
                    label: "doc_title".to_string(),
                    content: "Article Title".to_string(),
                    bbox: vec![80, 180, 600, 240],
                    order: Some(1),
                    page: Some(0),
                },
                crate::ocr::OcrLayoutBlock {
                    label: "text".to_string(),
                    content: "Alice Smith and Bob Jones".to_string(),
                    bbox: vec![80, 260, 500, 285],
                    order: Some(2),
                    page: Some(0),
                },
                crate::ocr::OcrLayoutBlock {
                    label: "text".to_string(),
                    content: "Department of A, University A and Department of B, University B"
                        .to_string(),
                    bbox: vec![80, 300, 700, 330],
                    order: Some(3),
                    page: Some(0),
                },
                crate::ocr::OcrLayoutBlock {
                    label: "paragraph_title".to_string(),
                    content: "Abstract".to_string(),
                    bbox: vec![80, 380, 180, 405],
                    order: Some(4),
                    page: Some(0),
                },
                crate::ocr::OcrLayoutBlock {
                    label: "text".to_string(),
                    content: "This body text must not be front matter.".to_string(),
                    bbox: vec![80, 420, 700, 470],
                    order: Some(5),
                    page: Some(0),
                },
            ],
        }];

        let blocks = first_page_header_layout_blocks(&page_layouts);

        assert_eq!(blocks.len(), 3);
        assert!(blocks[2].contains("Department of A"));
        assert!(!blocks.iter().any(|block| block.contains("body text")));
    }

    #[test]
    fn enriches_missing_affiliations_from_layout_context_by_author_order() {
        let candidate = HeaderBlockCandidate {
            blocks: vec![
                "First-page layout context:\nArticle Title\nAlice Smith and Bob Jones\nDepartment of A, University A and Department of B, University B".to_string(),
            ],
            probe_blocks: Vec::new(),
            body_start_block: 1,
            boundary: HeaderBoundary::FirstBodyParagraph,
        };
        let front_matter = FrontMatterExtraction {
            title: "Article Title".to_string(),
            authors: vec![
                FrontMatterAuthor {
                    name: "Alice Smith".to_string(),
                    markers: Vec::new(),
                    affiliations: Vec::new(),
                    email: None,
                    is_corresponding: false,
                    affiliation_refs: Vec::new(),
                    equal_contribution: Vec::new(),
                    author_notes: Vec::new(),
                    notes: Vec::new(),
                },
                FrontMatterAuthor {
                    name: "Bob Jones".to_string(),
                    markers: Vec::new(),
                    affiliations: Vec::new(),
                    email: None,
                    is_corresponding: false,
                    affiliation_refs: Vec::new(),
                    equal_contribution: Vec::new(),
                    author_notes: Vec::new(),
                    notes: Vec::new(),
                },
            ],
            extras: Vec::new(),
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };

        let enriched = enrich_front_matter_from_candidate(front_matter, &candidate);

        assert_eq!(
            enriched.authors[0].affiliations,
            vec!["Department of A, University A"]
        );
        assert_eq!(
            enriched.authors[1].affiliations,
            vec!["Department of B, University B"]
        );
    }

    #[test]
    fn builds_front_matter_candidate_with_layout_footnote() {
        let markdown = concat!(
            "# Article Title\n\n",
            "Jane Doe $ ^{1} $\n\n",
            "This is a long body paragraph with enough words to be treated as body text. ",
            "It keeps going until the detector sees a real paragraph instead of title metadata. ",
            "The sentence contains punctuation and many words for the heuristic."
        );
        let page_layouts = vec![OcrPageLayout {
            blocks: vec![crate::ocr::OcrLayoutBlock {
                label: "footnote".to_string(),
                content: "1 Department of Example. e-mail: jane@example.test".to_string(),
                bbox: vec![],
                order: Some(10),
                page: Some(0),
            }],
        }];

        let candidate = front_matter_candidate_from_layout(markdown, &page_layouts);

        assert!(candidate
            .blocks
            .iter()
            .any(|block| block.contains("First-page footnote")));
        assert!(candidate
            .blocks
            .iter()
            .any(|block| block.contains("jane@example.test")));
    }

    #[test]
    fn author_signal_detects_marker_or_footnote_context() {
        let candidate = HeaderBlockCandidate {
            blocks: vec!["Jane Doe $ ^{1} $".to_string()],
            probe_blocks: vec![],
            body_start_block: 1,
            boundary: HeaderBoundary::FirstBodyParagraph,
        };

        assert!(candidate_has_author_signal(&candidate));

        let plain = HeaderBlockCandidate {
            blocks: vec!["# Chapter One".to_string()],
            probe_blocks: vec![],
            body_start_block: 1,
            boundary: HeaderBoundary::FirstBodyParagraph,
        };

        assert!(!candidate_has_author_signal(&plain));
    }

    #[test]
    fn parses_and_sanitizes_front_matter_response() {
        let raw = concat!(
            "prefix ",
            "{\"title\":\"  Example Title  \",\"authors\":[",
            "{\"name\":\" Jane Doe \",\"markers\":[\" 1 \"],\"affiliations\":[\" Dept. \"],\"affiliationRefs\":[\" 1 \"],\"email\":\" jane@example.test \",\"isCorresponding\":true,\"equalContribution\":[\" contributed equally \"],\"authorNotes\":[\" note \"],\"notes\":[\" note \"]},",
            "{\"name\":\" \",\"markers\":[],\"affiliations\":[],\"email\":null,\"isCorresponding\":false,\"notes\":[]}",
            "],\"extras\":[\" DOI \",\"© The Author(s), 2021. Published by Cambridge University Press. This is an Open Access article distributed under the Creative Commons licence.\",\"\"],",
            "\"affiliationCatalog\":[{\"id\":\" 1 \",\"text\":\" Dept. \"}],",
            "\"extraEntries\":[{\"raw\":\" DOI \",\"kind\":\" doi \"},{\"raw\":\"© The Author(s), 2021. Published by Cambridge University Press. This is an Open Access article distributed under the Creative Commons licence.\",\"kind\":\" publishedDate \"}]}",
            " suffix"
        );

        let parsed = parse_front_matter_response(raw).expect("front matter");

        assert_eq!(parsed.title, "Example Title");
        assert_eq!(parsed.authors.len(), 1);
        assert_eq!(parsed.authors[0].name, "Jane Doe");
        assert_eq!(parsed.authors[0].markers, vec!["1"]);
        assert_eq!(parsed.authors[0].affiliations, vec!["Dept."]);
        assert_eq!(parsed.authors[0].affiliation_refs, vec!["1"]);
        assert_eq!(
            parsed.authors[0].email.as_deref(),
            Some("jane@example.test")
        );
        assert!(parsed.authors[0].is_corresponding);
        assert_eq!(
            parsed.authors[0].equal_contribution,
            vec!["contributed equally"]
        );
        assert_eq!(parsed.authors[0].author_notes, vec!["note"]);
        assert_eq!(parsed.authors[0].notes, vec!["note"]);
        assert_eq!(parsed.extras, vec!["DOI"]);
        assert_eq!(parsed.affiliation_catalog[0].id, "1");
        assert_eq!(parsed.affiliation_catalog[0].text, "Dept.");
        assert_eq!(parsed.extra_entries[0].raw, "DOI");
        assert_eq!(parsed.extra_entries[0].kind, "doi");
    }

    #[test]
    fn rewrites_source_markdown_with_structured_front_matter() {
        let markdown = concat!(
            "https://doi.org/10.1/example\n\n",
            "# Raw Title\n\n",
            "Jane Doe $ ^{1} $, John Roe $ ^{2,*} $\n\n",
            "Check for updates\n\n",
            "Abstract text starts here. This paragraph is intentionally long enough to trigger body detection because it contains many words and complete punctuation for the detector to identify the body boundary correctly."
        );
        let front_matter = AcademicFrontMatter {
            title: Some("Normalized Title".to_string()),
            authors: vec![
                AcademicAuthor {
                    name: "Jane Doe".to_string(),
                    email: None,
                    markers: vec!["1".to_string()],
                    affiliations: vec!["Department A".to_string()],
                    is_corresponding: false,
                    affiliation_refs: Vec::new(),
                    equal_contribution: Vec::new(),
                    author_notes: Vec::new(),
                    correspondence_note: None,
                },
                AcademicAuthor {
                    name: "John Roe".to_string(),
                    email: Some("john@example.test".to_string()),
                    markers: vec!["2".to_string(), "*".to_string()],
                    affiliations: vec!["Department B".to_string()],
                    is_corresponding: true,
                    affiliation_refs: Vec::new(),
                    equal_contribution: Vec::new(),
                    author_notes: Vec::new(),
                    correspondence_note: Some("Corresponding author".to_string()),
                },
            ],
            extras: vec!["https://doi.org/10.1/example".to_string()],
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };

        let rewritten = rewrite_source_markdown_with_front_matter(markdown, &front_matter);

        assert!(rewritten.starts_with(crate::pipeline::chunking::FRONT_MATTER_PROTECTED_START));
        assert!(rewritten.contains("# Normalized Title"));
        assert!(rewritten.contains(crate::pipeline::chunking::FRONT_MATTER_PROTECTED_END));
        assert!(rewritten.contains("Jane Doe $ ^{1} $, John Roe $ ^{2,*} $"));
        assert!(rewritten.contains("1 Department A"));
        assert!(rewritten.contains("2 Department B"));
        assert!(rewritten.contains("* Corresponding author"));
        assert!(rewritten.contains("Email: john@example.test"));
        assert!(rewritten.contains("https://doi.org/10.1/example"));
        assert!(!rewritten.contains("Check for updates"));
    }

    #[test]
    fn rewrite_source_front_matter_keeps_early_ocr_body_blocks() {
        let markdown = concat!(
            "communications psychology\n\n",
            "All analyzed data and questionnaires have been documented either in the manuscript or in the Supplementary Information.\n\n",
            "a)\n\n",
            "Proportion of correct judgments\n\n",
            "Ecology\n\n",
            "b)\n\n",
            "Proportion of news reported as being true\n\n",
            "Ecology\n\n",
            "## Demand and avoidance of extra information\n\n",
            "Participants exhibited a higher willingness-to-pay for receiving extra information.\n\n",
            "## Discussion\n\n",
            "Headlines in the real world often do not overtly appear true or false."
        );
        let front_matter = AcademicFrontMatter {
            title: Some(
                "Metacognition biases information seeking in assessing ambiguous news".to_string(),
            ),
            authors: vec![
                AcademicAuthor {
                    name: "Valentin Guigon".to_string(),
                    email: None,
                    markers: vec!["1".to_string(), "2".to_string()],
                    affiliations: vec!["Neuroeconomics lab".to_string(), "GATE".to_string()],
                    is_corresponding: false,
                    affiliation_refs: vec!["1".to_string(), "2".to_string()],
                    equal_contribution: Vec::new(),
                    author_notes: Vec::new(),
                    correspondence_note: None,
                },
                AcademicAuthor {
                    name: "Jean-Claude Dreher".to_string(),
                    email: Some("dreher@isc.cnrs.fr".to_string()),
                    markers: vec!["1".to_string()],
                    affiliations: vec!["Neuroeconomics lab".to_string()],
                    is_corresponding: true,
                    affiliation_refs: vec!["1".to_string()],
                    equal_contribution: Vec::new(),
                    author_notes: Vec::new(),
                    correspondence_note: None,
                },
            ],
            extras: vec![
                "https://doi.org/10.1038/s44271-024-00170-w".to_string(),
                "communications psychology".to_string(),
                "Article".to_string(),
            ],
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };

        let rewritten = rewrite_source_markdown_with_front_matter(markdown, &front_matter);

        assert!(rewritten.contains("All analyzed data and questionnaires have been documented"));
        assert!(rewritten.contains("## Demand and avoidance of extra information"));
        assert!(rewritten.contains("## Discussion"));
        assert!(
            rewritten
                .find("All analyzed data and questionnaires")
                .expect("early OCR block should remain")
                < rewritten
                    .find("## Demand and avoidance of extra information")
                    .expect("section heading should remain")
        );
    }

    #[test]
    fn front_matter_contract_enrichment_preserves_legacy_fields() {
        let front_matter = FrontMatterExtraction {
            title: "Sample Title".to_string(),
            authors: vec![FrontMatterAuthor {
                name: "Author A".to_string(),
                markers: vec!["1".to_string()],
                affiliations: vec!["Dept. of Testing".to_string()],
                affiliation_refs: vec!["1".to_string()],
                email: None,
                is_corresponding: false,
                equal_contribution: vec!["These authors contributed equally.".to_string()],
                author_notes: vec!["These authors contributed equally.".to_string()],
                notes: vec!["These authors contributed equally.".to_string()],
            }],
            extras: vec!["https://doi.org/10.1/example".to_string()],
            affiliation_catalog: vec![FrontMatterAffiliationEntry {
                id: "1".to_string(),
                text: "Dept. of Testing".to_string(),
            }],
            extra_entries: vec![FrontMatterExtraEntry {
                raw: "https://doi.org/10.1/example".to_string(),
                kind: "doi".to_string(),
            }],
        };

        let academic = front_matter_to_academic_front_matter(front_matter);

        assert_eq!(academic.title.as_deref(), Some("Sample Title"));
        assert_eq!(academic.extras, vec!["https://doi.org/10.1/example"]);
        assert_eq!(
            academic.authors[0].correspondence_note.as_deref(),
            Some("These authors contributed equally.")
        );
        assert_eq!(
            academic.authors[0].author_notes,
            vec!["These authors contributed equally."]
        );
        assert_eq!(
            academic.authors[0].equal_contribution,
            vec!["These authors contributed equally."]
        );
        assert_eq!(academic.affiliation_catalog.len(), 1);
        assert_eq!(academic.affiliation_catalog[0].text, "Dept. of Testing");
        assert_eq!(academic.extra_entries[0].kind, "doi");
    }

    #[test]
    fn front_matter_uses_affiliation_catalog_refs_for_author_popovers() {
        let front_matter = FrontMatterExtraction {
            title: "Sample Title".to_string(),
            authors: vec![FrontMatterAuthor {
                name: "Jane Doe".to_string(),
                markers: vec!["1".to_string()],
                affiliations: Vec::new(),
                affiliation_refs: vec!["1".to_string()],
                email: Some("jane@example.test".to_string()),
                is_corresponding: true,
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                notes: Vec::new(),
            }],
            extras: Vec::new(),
            affiliation_catalog: vec![FrontMatterAffiliationEntry {
                id: "1".to_string(),
                text: "Department of Testing, Example University".to_string(),
            }],
            extra_entries: Vec::new(),
        };

        let academic = front_matter_to_academic_front_matter(front_matter);

        assert_eq!(
            academic.authors[0].affiliations,
            vec!["Department of Testing, Example University"]
        );
        assert_eq!(academic.affiliation_catalog.len(), 1);
        assert_eq!(academic.affiliation_catalog[0].id, "1");
    }

    #[test]
    fn front_matter_translation_repair_detects_untranslated_affiliations() {
        let source = AcademicFrontMatter {
            title: Some("Example Article".to_string()),
            authors: vec![AcademicAuthor {
                name: "Jane Doe".to_string(),
                email: Some("jane@example.test".to_string()),
                markers: vec!["1".to_string()],
                affiliations: vec![
                    "Department of Testing, Example University, London, UK".to_string()
                ],
                is_corresponding: true,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: Some("Email: jane@example.test".to_string()),
            }],
            extras: vec!["doi:10.1/example".to_string()],
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };
        let translated = source.clone();

        assert!(front_matter_needs_translation_repair(&source, &translated));
    }

    #[test]
    fn front_matter_translation_repair_detects_split_untranslated_affiliations() {
        let source = AcademicFrontMatter {
            title: Some("Nudge plus".to_string()),
            authors: vec![
                AcademicAuthor {
                    name: "Sanchayan Banerjee".to_string(),
                    email: None,
                    markers: Vec::new(),
                    affiliations: vec![
                        "Department of Geography and Environment, London School of Economics, London, UK and Department of Political Economy, King's College London, London, UK"
                            .to_string(),
                    ],
                    is_corresponding: false,
                    affiliation_refs: Vec::new(),
                    equal_contribution: Vec::new(),
                    author_notes: Vec::new(),
                    correspondence_note: None,
                },
                AcademicAuthor {
                    name: "Peter John".to_string(),
                    email: Some("peter.john@kcl.ac.uk".to_string()),
                    markers: vec!["*".to_string()],
                    affiliations: vec![
                        "Department of Geography and Environment, London School of Economics, London, UK and Department of Political Economy, King's College London, London, UK"
                            .to_string(),
                    ],
                    is_corresponding: true,
                    affiliation_refs: Vec::new(),
                    equal_contribution: Vec::new(),
                    author_notes: Vec::new(),
                    correspondence_note: None,
                },
            ],
            extras: Vec::new(),
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };
        let translated = AcademicFrontMatter {
            title: Some("\u{52A9}\u{63A8}+".to_string()),
            authors: vec![
                AcademicAuthor {
                    name: "Sanchayan Banerjee".to_string(),
                    email: None,
                    markers: Vec::new(),
                    affiliations: vec![
                        "Department of Geography and Environment, London School of Economics, London, UK"
                            .to_string(),
                    ],
                    is_corresponding: false,
                    affiliation_refs: Vec::new(),
                    equal_contribution: Vec::new(),
                    author_notes: Vec::new(),
                    correspondence_note: None,
                },
                AcademicAuthor {
                    name: "Peter John".to_string(),
                    email: Some("peter.john@kcl.ac.uk".to_string()),
                    markers: vec!["*".to_string()],
                    affiliations: vec![
                        "Department of Political Economy, King's College London, London, UK"
                            .to_string(),
                    ],
                    is_corresponding: true,
                    affiliation_refs: Vec::new(),
                    equal_contribution: Vec::new(),
                    author_notes: Vec::new(),
                    correspondence_note: None,
                },
            ],
            extras: Vec::new(),
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };

        assert!(front_matter_needs_translation_repair(&source, &translated));
    }

    #[test]
    fn front_matter_translation_repair_accepts_translated_affiliations_and_preserved_ids() {
        let source = AcademicFrontMatter {
            title: Some("Example Article".to_string()),
            authors: vec![AcademicAuthor {
                name: "Jane Doe".to_string(),
                email: Some("jane@example.test".to_string()),
                markers: vec!["1".to_string()],
                affiliations: vec![
                    "Department of Testing, Example University, London, UK".to_string()
                ],
                is_corresponding: true,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: Some("Email: jane@example.test".to_string()),
            }],
            extras: vec!["doi:10.1/example".to_string()],
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };
        let translated = AcademicFrontMatter {
            title: Some("示例文章".to_string()),
            authors: vec![AcademicAuthor {
                name: "Jane Doe".to_string(),
                email: Some("jane@example.test".to_string()),
                markers: vec!["1".to_string()],
                affiliations: vec!["测试系，示例大学，英国伦敦".to_string()],
                is_corresponding: true,
                affiliation_refs: Vec::new(),
                equal_contribution: Vec::new(),
                author_notes: Vec::new(),
                correspondence_note: Some("Email: jane@example.test".to_string()),
            }],
            extras: vec!["doi:10.1/example".to_string()],
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };

        assert!(!front_matter_needs_translation_repair(&source, &translated));
    }

    #[test]
    fn front_matter_splits_repeated_unmarked_affiliations_by_author_order() {
        let affiliations = vec![
            "Department of Geography and Environment, London School of Economics, London, UK"
                .to_string(),
            "Department of Political Economy, King's College London, London, UK".to_string(),
        ];
        let front_matter = FrontMatterExtraction {
            title: "Nudge plus".to_string(),
            authors: vec![
                FrontMatterAuthor {
                    name: "Sanchayan Banerjee".to_string(),
                    markers: Vec::new(),
                    affiliations: affiliations.clone(),
                    affiliation_refs: Vec::new(),
                    email: None,
                    is_corresponding: false,
                    equal_contribution: Vec::new(),
                    author_notes: Vec::new(),
                    notes: Vec::new(),
                },
                FrontMatterAuthor {
                    name: "Peter John".to_string(),
                    markers: vec!["*".to_string()],
                    affiliations: affiliations.clone(),
                    affiliation_refs: Vec::new(),
                    email: Some("peter.john@kcl.ac.uk".to_string()),
                    is_corresponding: true,
                    equal_contribution: Vec::new(),
                    author_notes: Vec::new(),
                    notes: Vec::new(),
                },
            ],
            extras: Vec::new(),
            affiliation_catalog: Vec::new(),
            extra_entries: Vec::new(),
        };

        let academic = front_matter_to_academic_front_matter(front_matter);

        assert_eq!(
            academic.authors[0].affiliations,
            vec![affiliations[0].clone()]
        );
        assert_eq!(
            academic.authors[1].affiliations,
            vec![affiliations[1].clone()]
        );
        assert_eq!(academic.affiliation_catalog.len(), 2);
    }

    #[test]
    fn corresponding_author_marker_is_not_affiliation_ref() {
        let authors = vec![AcademicAuthor {
            name: "Peter John".to_string(),
            email: Some("peter@example.test".to_string()),
            markers: vec!["*".to_string()],
            affiliations: vec!["Department of Testing".to_string()],
            is_corresponding: true,
            affiliation_refs: Vec::new(),
            equal_contribution: Vec::new(),
            author_notes: Vec::new(),
            correspondence_note: Some("Corresponding author".to_string()),
        }];

        let catalog = build_affiliation_catalog(&authors);
        let detail_lines = render_source_author_detail_lines(&authors);

        assert_eq!(catalog[0].id, "Department of Testing");
        assert!(detail_lines
            .iter()
            .any(|line| line == "Department of Testing"));
        assert!(detail_lines
            .iter()
            .any(|line| line == "* Corresponding author"));
        assert!(!detail_lines
            .iter()
            .any(|line| line == "* Department of Testing"));
    }
}
