use std::path::Path;

pub(crate) fn join_epub_path(base_dir: &str, href: &str) -> String {
    let mut base = std::path::PathBuf::from(base_dir);
    base.push(href.split('#').next().unwrap_or(href));
    normalize_epub_path(&normalize_path_string(&base))
}

pub(crate) fn normalize_href_key(raw: &str) -> String {
    normalize_epub_path(
        raw.split('#')
            .next()
            .unwrap_or(raw)
            .trim_start_matches("./"),
    )
}

pub(crate) fn parent_dir(path: &str) -> String {
    Path::new(path)
        .parent()
        .map(normalize_path_string)
        .unwrap_or_default()
}

pub(crate) fn normalize_epub_path(raw: &str) -> String {
    let mut parts = Vec::new();
    let normalized = raw.replace('\\', "/");
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    parts.join("/")
}

fn normalize_path_string(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_epub_path_resolves_parent_segments() {
        assert_eq!(
            join_epub_path("OEBPS/nav", "../Text/chapter.xhtml#top"),
            "OEBPS/Text/chapter.xhtml"
        );
    }

    #[test]
    fn normalize_href_key_strips_fragment_and_dot_segments() {
        assert_eq!(
            normalize_href_key("./nav/../Text/chapter.xhtml#section"),
            "Text/chapter.xhtml"
        );
    }
}
