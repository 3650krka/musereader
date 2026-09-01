mod academic;
mod reference;

pub(crate) use academic::{
    looks_like_protected_metadata_chunk, FRONT_MATTER_PROTECTED_END, FRONT_MATTER_PROTECTED_START,
};
pub(crate) use reference::{
    is_non_reference_tail_heading, looks_like_reference_chunk, should_skip_residual_review,
    split_reference_aware_markdown, starts_reference_section,
    starts_with_non_reference_tail_heading,
};

pub(crate) fn split_markdown(text: &str, chunk_size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current_chunk = String::new();
    let mut table_block = String::new();
    let mut in_pdf_table_block = false;

    for line in text.lines() {
        if line.trim() == super::layout::PDF_TABLE_START_MARKER {
            flush_chunk(&mut chunks, &mut current_chunk);
            in_pdf_table_block = true;
            table_block.push_str(line);
            table_block.push('\n');
            continue;
        }

        if in_pdf_table_block {
            table_block.push_str(line);
            table_block.push('\n');
            if line.trim() == super::layout::PDF_TABLE_END_MARKER {
                chunks.push(table_block.clone());
                table_block.clear();
                in_pdf_table_block = false;
            }
            continue;
        }

        if is_markdown_image_line(line) {
            flush_chunk(&mut chunks, &mut current_chunk);
            chunks.push(format!("{}\n", line.trim_end()));
            continue;
        }

        if current_chunk.len() + line.len() > chunk_size && !current_chunk.is_empty() {
            flush_chunk(&mut chunks, &mut current_chunk);
        }
        current_chunk.push_str(line);
        current_chunk.push('\n');
    }

    if !table_block.is_empty() {
        chunks.push(table_block);
    }
    if !current_chunk.is_empty() {
        flush_chunk(&mut chunks, &mut current_chunk);
    }

    chunks
}

fn flush_chunk(chunks: &mut Vec<String>, current_chunk: &mut String) {
    if current_chunk.is_empty() {
        return;
    }
    chunks.push(current_chunk.clone());
    current_chunk.clear();
}

pub(crate) fn is_academic_article(article_type: &str) -> bool {
    crate::article_policy::ArticlePolicy::from_article_type(article_type).is_academic()
}

pub(crate) fn is_academic_article_from_markdown(markdown: &str) -> bool {
    let lower = markdown.to_ascii_lowercase();
    lower.contains("\n## references")
        || lower.contains("\n## abstract")
        || lower.contains("\nkeywords:")
        || lower.contains("\nreceived ")
        || lower.contains("\naccepted ")
        || lower.contains("\ndoi")
}

pub(crate) fn is_markdown_image_chunk(text: &str) -> bool {
    let mut saw_image = false;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if !is_markdown_image_line(line) {
            return false;
        }
        saw_image = true;
    }
    saw_image
}

fn is_markdown_image_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("![") && trimmed.contains("](") && trimmed.ends_with(')')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::layout;

    #[test]
    fn pdf_table_blocks_are_split_as_standalone_chunks() {
        let markdown = format!(
            "Before\n\n{}\n![PDF table 1](imgs/table_snapshot_001.jpg)\n{}\n\nAfter",
            layout::PDF_TABLE_START_MARKER,
            layout::PDF_TABLE_END_MARKER
        );

        let chunks = split_markdown(&markdown, 3000);

        assert_eq!(chunks.len(), 3);
        assert!(chunks[0].contains("Before"));
        assert!(layout::is_pdf_table_block(&chunks[1]));
        assert!(chunks[1].contains("![PDF table 1](imgs/table_snapshot_001.jpg)"));
        assert!(chunks[2].contains("After"));
    }

    #[test]
    fn markdown_images_are_split_as_standalone_chunks() {
        let markdown = "Before\n\n![Cover](data:image/png;base64,abc)\n\nAfter";

        let chunks = split_markdown(markdown, 3000);

        assert_eq!(chunks.len(), 3);
        assert!(chunks[0].contains("Before"));
        assert!(is_markdown_image_chunk(&chunks[1]));
        assert!(chunks[2].contains("After"));
    }
}
