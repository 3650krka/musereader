use super::split_markdown;

pub(crate) const FRONT_MATTER_PROTECTED_START: &str = "<!-- musetranslate:front-matter:start -->";
pub(crate) const FRONT_MATTER_PROTECTED_END: &str = "<!-- musetranslate:front-matter:end -->";

pub(crate) fn looks_like_protected_metadata_chunk(text: &str) -> bool {
    let trimmed = text.trim();
    if is_explicit_front_matter_region(trimmed) {
        return true;
    }
    if trimmed.is_empty() || !is_front_matter_region(trimmed) || contains_substantive_prose(trimmed)
    {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    has_academic_author_marker(trimmed)
        || lower.contains("## addresses")
        || lower.contains("corresponding authors:")
        || lower.contains("corresponding author:")
        || lower.contains("department of ")
        || lower.contains("university of ")
        || trimmed.lines().filter(|line| line.contains('@')).count() >= 1
}

pub(crate) fn split_academic_body_and_metadata_chunks(
    text: &str,
    chunk_size: usize,
) -> Vec<String> {
    let mut state = AcademicChunkState::default();
    for line in text.lines() {
        state.consume_line(line, chunk_size);
    }
    state.finish(chunk_size)
}

struct AcademicChunkState {
    chunks: Vec<String>,
    body: String,
    protected: String,
    in_addresses: bool,
    front_matter_open: bool,
    explicit_front_matter_open: bool,
    saw_title_heading: bool,
}

impl AcademicChunkState {
    fn consume_line(&mut self, line: &str, chunk_size: usize) {
        let trimmed = line.trim();
        if trimmed == FRONT_MATTER_PROTECTED_START {
            self.flush_body(chunk_size);
            self.front_matter_open = true;
            self.explicit_front_matter_open = true;
            push_line(&mut self.protected, line);
            return;
        }
        if self.explicit_front_matter_open {
            push_line(&mut self.protected, line);
            if trimmed == FRONT_MATTER_PROTECTED_END {
                self.explicit_front_matter_open = false;
                self.front_matter_open = false;
                self.flush_protected();
            }
            return;
        }
        if self.front_matter_open && is_markdown_heading_label(trimmed, "addresses") {
            self.flush_body(chunk_size);
            self.in_addresses = true;
            push_line(&mut self.protected, line);
            return;
        }
        if trimmed.starts_with('#') && !is_markdown_heading_label(trimmed, "addresses") {
            self.close_addresses_block();
            if self.front_matter_open {
                if self.saw_title_heading {
                    self.front_matter_open = false;
                    self.flush_protected();
                } else {
                    self.saw_title_heading = true;
                }
            }
        }
        if self.in_addresses {
            self.flush_body(chunk_size);
            push_line(&mut self.protected, line);
            return;
        }
        if self.front_matter_open && is_protected_academic_metadata_line(line) {
            self.flush_body(chunk_size);
            push_line(&mut self.protected, line);
            return;
        }
        if self.front_matter_open && looks_like_front_matter_body_start(trimmed) {
            self.front_matter_open = false;
            self.flush_protected();
        }
        self.flush_protected();
        push_line(&mut self.body, line);
    }

    fn finish(mut self, chunk_size: usize) -> Vec<String> {
        self.flush_body(chunk_size);
        self.flush_protected();
        self.chunks
    }

    fn close_addresses_block(&mut self) {
        if self.in_addresses {
            self.flush_protected();
        }
        self.in_addresses = false;
    }

    fn flush_body(&mut self, chunk_size: usize) {
        if self.body.trim().is_empty() {
            self.body.clear();
            return;
        }
        self.chunks
            .extend(split_markdown(self.body.trim_end(), chunk_size));
        self.body.clear();
    }

    fn flush_protected(&mut self) {
        if self.protected.trim().is_empty() {
            self.protected.clear();
            return;
        }
        self.chunks.push(self.protected.trim_end().to_string());
        self.protected.clear();
    }
}

impl Default for AcademicChunkState {
    fn default() -> Self {
        Self {
            chunks: Vec::new(),
            body: String::new(),
            protected: String::new(),
            in_addresses: false,
            front_matter_open: true,
            explicit_front_matter_open: false,
            saw_title_heading: false,
        }
    }
}

fn push_line(target: &mut String, line: &str) {
    target.push_str(line);
    target.push('\n');
}

fn is_explicit_front_matter_region(text: &str) -> bool {
    text.contains(FRONT_MATTER_PROTECTED_START) && text.contains(FRONT_MATTER_PROTECTED_END)
}

fn is_protected_academic_metadata_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    has_academic_author_marker(trimmed)
        || lower.starts_with("corresponding authors:")
        || lower.starts_with("corresponding author:")
        || trimmed.contains('@')
        || trimmed.starts_with("$ ^{")
}

fn is_markdown_heading_label(line: &str, expected: &str) -> bool {
    let normalized = line
        .trim()
        .trim_start_matches('#')
        .trim()
        .trim_end_matches(':')
        .to_ascii_lowercase();
    normalized == expected
}

fn looks_like_front_matter_body_start(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty()
        && !trimmed.starts_with('#')
        && !is_protected_academic_metadata_line(trimmed)
        && looks_like_substantive_prose_line(trimmed)
}

fn is_front_matter_region(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    if lower.contains("\n## introduction") || lower.contains("\n# introduction") {
        return false;
    }
    let heading_count = text
        .lines()
        .filter(|line| line.trim_start().starts_with('#'))
        .count();
    text.chars().count() <= 2800 && heading_count <= 5
}

fn contains_substantive_prose(text: &str) -> bool {
    text.lines().any(looks_like_substantive_prose_line)
}

fn looks_like_substantive_prose_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 80
        || !contains_prose_sentence_punctuation(trimmed)
        || trimmed.contains('@')
        || trimmed.starts_with('#')
    {
        return false;
    }

    let word_count = trimmed
        .split_whitespace()
        .filter(|token| token.chars().any(|ch| ch.is_ascii_alphabetic()))
        .count();
    word_count >= 12
}

fn has_academic_author_marker(text: &str) -> bool {
    text.lines().any(looks_like_author_line)
}

fn looks_like_author_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.len() > 240
        || !contains_author_superscript_marker(trimmed)
        || contains_prose_sentence_punctuation(trimmed)
    {
        return false;
    }

    let normalized = strip_superscript_markers(trimmed);
    let tokens = normalized
        .split(|ch: char| !(ch.is_ascii_alphabetic() || ch == '\'' || ch == '-'))
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.len() < 2 || tokens.len() > 18 {
        return false;
    }

    let capitalized_tokens = tokens
        .iter()
        .filter(|token| is_author_name_token(token))
        .count();
    let lowercase_word_tokens = tokens
        .iter()
        .filter(|token| {
            token.len() >= 4
                && token
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_lowercase())
        })
        .count();

    capitalized_tokens >= 2 && lowercase_word_tokens <= 2
}

fn contains_author_superscript_marker(line: &str) -> bool {
    line.contains("$ ^{") || line.contains("\\textcircled{D}^{")
}

fn contains_prose_sentence_punctuation(line: &str) -> bool {
    line.contains('.')
        || line.contains('?')
        || line.contains('!')
        || line.contains(':')
        || line.contains(';')
}

fn strip_superscript_markers(line: &str) -> String {
    let mut normalized = line.replace("\\textcircled{D}", " ");
    while let Some(start) = normalized.find("$ ^{") {
        let Some(end_rel) = normalized[start..].find("} $") else {
            break;
        };
        let end = start + end_rel + 3;
        normalized.replace_range(start..end, " ");
    }
    normalized
}

fn is_author_name_token(token: &&str) -> bool {
    let mut chars = token.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_uppercase()
        && chars.all(|ch| ch.is_ascii_alphabetic() || ch == '\'' || ch == '-')
}

#[cfg(test)]
mod tests {
    use super::{
        looks_like_author_line, looks_like_protected_metadata_chunk,
        split_academic_body_and_metadata_chunks, FRONT_MATTER_PROTECTED_END,
        FRONT_MATTER_PROTECTED_START,
    };

    #[test]
    fn author_line_detection_accepts_real_author_block() {
        let line = "Valentin Guigon $ ^{1,2} $, Marie Claire Villeval $ \\textcircled{D}^{2,3,4} $ & Jean-Claude Dreher $ \\textcircled{D}^{1,4} $";

        assert!(looks_like_author_line(line));
        assert!(looks_like_protected_metadata_chunk(line));
    }

    #[test]
    fn author_line_detection_rejects_body_sentence_with_citations() {
        let line = "Epistemic curiosity may respond to the desire to stimulate positive feelings of intellectual interest or the desire to reduce undesirable states of information deprivation $ ^{56} $.";

        assert!(!looks_like_author_line(line));
        assert!(!looks_like_protected_metadata_chunk(line));
    }

    #[test]
    fn author_line_detection_rejects_long_academic_body_paragraph() {
        let line = "At the end of the experiment, participants had to fill in several questionnaires allowing us to measure notably their exposition of information and their degree of curiosity (see Supplementary Methods IV).";

        assert!(!looks_like_author_line(line));
        assert!(!looks_like_protected_metadata_chunk(line));
    }

    #[test]
    fn protected_metadata_detection_rejects_real_methods_paragraphs() {
        let paragraphs = [
            "Individuals' worldviews have been shown to explain what they believe to be true $ ^{41} $. As a proxy for beliefs, we adapted measures of social distance between individuals to the relationships individuals may maintain with organizations. In our context, a closer social distance would mean more involvement in the concerns related to the themes.",
            "The second part of the experiment involved a two-stage task (Supplementary Methods II.2). The first stage included the veracity judgment task. Participants were divided into two groups that received 48 different stimuli each.",
            "The second stage corresponded to the elicitation of the demand to receive extra information. After validating their veracity judgment and while their screen was still displaying the brief news, participants were asked to choose between receiving or not additional information related to the same news after the completion of the experiment.",
            "At the end of the experiment, participants had to fill in several questionnaires allowing us to measure notably their exposition of information and their degree of curiosity (see Supplementary Methods IV). Epistemic curiosity may respond to the desire to stimulate positive feelings of intellectual interest or the desire to reduce undesirable states of information deprivation $ ^{56} $.",
            "We computed power for first-wave (N = 79) data and simulated power for sample sizes of up to 250 participants. We employed Mixed Linear Models (MLMs) of the confidence hypothesis, controlled for the veracity judgment and the interaction of news veracity with news theme.",
        ];

        for paragraph in paragraphs {
            assert!(
                !looks_like_protected_metadata_chunk(paragraph),
                "paragraph should stay translatable body: {paragraph}"
            );
        }
    }

    #[test]
    fn protected_metadata_detection_accepts_explicit_front_matter_region() {
        let markdown = [
            FRONT_MATTER_PROTECTED_START,
            "# Paper title",
            "Jane Doe",
            "Department of Testing, Example University",
            FRONT_MATTER_PROTECTED_END,
        ]
        .join("\n");

        assert!(looks_like_protected_metadata_chunk(&markdown));
    }

    #[test]
    fn splitting_keeps_explicit_front_matter_protected_without_abstract() {
        let markdown = [
            FRONT_MATTER_PROTECTED_START,
            "# Paper title",
            "",
            "Jane Doe, John Roe",
            "Department of Testing, Example University",
            FRONT_MATTER_PROTECTED_END,
            "",
            "The opening paragraph starts the article body without an abstract heading, and it should remain translatable body content rather than protected metadata.",
        ]
        .join("\n");

        let chunks = split_academic_body_and_metadata_chunks(&markdown, 240);

        assert!(chunks
            .iter()
            .any(|chunk| chunk.contains("Jane Doe") && looks_like_protected_metadata_chunk(chunk)));
        assert!(chunks
            .iter()
            .any(|chunk| chunk.contains("The opening paragraph starts")
                && !looks_like_protected_metadata_chunk(chunk)));
    }

    #[test]
    fn splitting_keeps_author_block_protected_but_releases_following_body() {
        let markdown = [
            "# Paper title",
            "",
            "Valentin Guigon $ ^{1,2} $, Marie Claire Villeval $ \\textcircled{D}^{2,3,4} $",
            "University of Lyon, France",
            "contact@example.edu",
            "",
            "The second stage corresponded to the elicitation of the demand to receive extra information. After validating their veracity judgment and while their screen was still displaying the brief news, participants were asked to choose between receiving or not additional information related to the same news after the completion of the experiment.",
        ]
        .join("\n");

        let chunks = split_academic_body_and_metadata_chunks(&markdown, 240);

        assert!(chunks
            .iter()
            .any(|chunk| chunk.contains("contact@example.edu")
                && looks_like_protected_metadata_chunk(chunk)));
        assert!(chunks
            .iter()
            .any(|chunk| chunk.contains("The second stage corresponded")
                && !looks_like_protected_metadata_chunk(chunk)));
    }
}
