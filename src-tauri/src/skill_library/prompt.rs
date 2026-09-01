pub(super) fn remove_frontmatter(content: &str) -> String {
    let lines = content.lines().collect::<Vec<_>>();
    if lines.first().is_some_and(|line| line.trim() == "---") {
        for (index, line) in lines.iter().enumerate().skip(1) {
            if line.trim() == "---" {
                return lines[index + 1..].join("\n").trim().to_string();
            }
        }
    }
    content.trim().to_string()
}

pub(super) fn build_translation_system_prompt(
    article_type: &str,
    core_content: &str,
    reference_content: &str,
) -> String {
    let core_body = trim_article_type_rules_section(core_content);
    [
        "# 翻译 Agent 系统提示",
        "",
        &format!("当前文章类型: {article_type}"),
        "",
        "## 核心翻译 Skill",
        "",
        &core_body,
        "",
        "## 当前类型专用规则",
        "",
        reference_content.trim(),
    ]
    .join("\n")
}

pub(super) fn extract_title(content: &str) -> String {
    let body = remove_frontmatter(content);
    body.lines()
        .find_map(|line| line.trim().strip_prefix("# ").map(str::trim))
        .filter(|line| !line.is_empty())
        .unwrap_or("未命名 Skill")
        .to_string()
}

pub(super) fn extract_description(content: &str) -> String {
    let body = remove_frontmatter(content);
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("无描述")
        .to_string()
}

pub(super) fn stable_hash_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn trim_article_type_rules_section(content: &str) -> String {
    let marker = "## 文章类型专用规则";
    match content.find(marker) {
        Some(index) => content[..index].trim().to_string(),
        None => content.trim().to_string(),
    }
}
