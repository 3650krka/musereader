#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpeningTitleAction {
    Advance,
    RemoveCurrent,
}

pub fn sanitize_epub_chapter_markdown(markdown: &str) -> String {
    let mut blocks = split_markdown_blocks(markdown);
    remove_repeated_opening_titles(&mut blocks);
    blocks.join("\n\n")
}

pub fn sanitize_translated_markdown(markdown: &str) -> String {
    let mut blocks = split_markdown_blocks(markdown);
    remove_repeated_opening_titles(&mut blocks);
    blocks.join("\n\n")
}

fn split_markdown_blocks(markdown: &str) -> Vec<String> {
    markdown
        .split("\n\n")
        .map(str::trim)
        .filter(|block| !block.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn remove_repeated_opening_titles(blocks: &mut Vec<String>) {
    let Some(title) = first_markdown_heading(blocks) else {
        return;
    };

    let mut index = 0usize;
    while index < blocks.len() {
        match opening_title_action(blocks, index, &title) {
            OpeningTitleAction::Advance => index += 1,
            OpeningTitleAction::RemoveCurrent => {
                blocks.remove(index);
            }
        }
    }
}

fn opening_title_action(blocks: &[String], index: usize, title: &str) -> OpeningTitleAction {
    if !is_duplicate_title_block(&blocks[index], title) {
        return OpeningTitleAction::Advance;
    }

    if is_comment_block(&blocks[index]) {
        return OpeningTitleAction::Advance;
    }

    if index == 0 {
        return OpeningTitleAction::Advance;
    }

    if should_remove_front_matter_title_pair(&blocks[index], blocks.get(index + 1), title)
        || is_short_front_matter_label(&blocks[index])
    {
        return OpeningTitleAction::RemoveCurrent;
    }

    OpeningTitleAction::Advance
}

fn should_remove_front_matter_title_pair(
    current: &str,
    next: Option<&String>,
    title: &str,
) -> bool {
    is_duplicate_title_block(current, title)
        && !is_comment_block(current)
        && next
            .map(|next| !is_comment_block(next) && is_duplicate_title_block(next, title))
            .unwrap_or(false)
}

fn is_comment_block(block: &str) -> bool {
    block.trim_start().starts_with("<!--")
}

fn first_markdown_heading(blocks: &[String]) -> Option<String> {
    blocks.iter().find_map(|block| block_heading_text(block))
}

fn block_heading_text(block: &str) -> Option<String> {
    block.lines().find_map(|line| {
        let trimmed = line.trim();
        trimmed
            .starts_with('#')
            .then_some(trimmed.trim_start_matches('#').trim().to_string())
    })
}

fn is_duplicate_title_block(block: &str, title: &str) -> bool {
    normalize_title(block) == normalize_title(title)
}

fn is_short_front_matter_label(block: &str) -> bool {
    let trimmed = block.trim();
    trimmed.chars().count() <= 80
        && !trimmed.ends_with('.')
        && !trimmed.ends_with('\u{3002}')
        && trimmed
            .split_whitespace()
            .filter(|part| !part.is_empty())
            .count()
            <= 8
}

fn normalize_title(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('#')
        .trim()
        .chars()
        .filter(|ch| !ch.is_whitespace() && !matches!(ch, ':' | '\u{ff1a}' | '-' | '\u{2014}'))
        .flat_map(char::to_lowercase)
        .collect()
}
