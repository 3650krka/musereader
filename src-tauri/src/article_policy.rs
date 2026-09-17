//! 文章类型策略（ArticlePolicy）：把散落各处的 article_type 分支集中为
//! 单一解析点 + 统一查询接口。各处不再各自 `eq_ignore_ascii_case("fiction")`
//! 或维护重复的 narrative/specialized 名单——全部改为询问本模块。
//!
//! 分类（与 11 个合法类型一一对应，见 skill_library::ARTICLE_TYPES）：
//! - Academic：academic
//! - Specialized：business / legal / tech_doc / textbook / nonfiction
//! - Narrative：fiction / children / blog / news / general（未知类型亦归入此类，
//!   与此前 detect.rs 的 `_ => Narrative` 防御性默认一致）
//!
//! 大小写归一在此一次完成（trim + lowercase）。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArticlePolicy {
    Narrative,
    Specialized,
    Academic,
}

impl ArticlePolicy {
    /// 唯一解析入口：大小写不敏感；未知类型按 Narrative 处理（保守默认，
    /// 与历史行为一致）。入口校验（lifecycle::validate_article_type）保证
    /// 生产路径不会收到未知类型；此默认仅作纵深防御。
    pub fn from_article_type(article_type: &str) -> Self {
        match article_type.trim().to_ascii_lowercase().as_str() {
            "academic" => Self::Academic,
            "business" | "legal" | "tech_doc" | "textbook" | "nonfiction" => Self::Specialized,
            _ => Self::Narrative,
        }
    }

    pub fn is_academic(self) -> bool {
        matches!(self, Self::Academic)
    }

    /// 叙事类文本（fiction/children/blog/news/general）：
    /// - 人名/地名等故事世界专有名词做本地化音译，并跨 chunk 传播
    /// - 残留英文检测允许保留 TitleCase 专有名词（仅 blog/news/general 等现实叙事文本；
    ///   故事世界文本 fiction/children 强制音译，TitleCase 不豁免，见 detect.rs）
    pub fn is_narrative(self) -> bool {
        matches!(self, Self::Narrative)
    }

    /// 专业类文本（academic + business/legal/tech_doc/textbook/nonfiction）：
    /// - 残留英文检测额外屏蔽参考文献、DOI/ISBN 等专业区段
    pub fn is_specialized_or_academic(self) -> bool {
        matches!(self, Self::Academic | Self::Specialized)
    }

    /// 故事世界规则（仅 fiction/children）：运行时注入故事世界翻译规则
    /// （角色名必须音译、confirmedTerms 记录全部故事世界名词等）。
    /// 比 is_narrative 更窄——blog/news/general 是叙事文本但没有"故事世界"。
    pub fn has_story_world(article_type: &str) -> bool {
        matches!(
            article_type.trim().to_ascii_lowercase().as_str(),
            "fiction" | "children"
        )
    }

    /// person-term 接受判定（需要原始类型串区分 specialized 细分）。
    pub fn accepts_person_terms_for(article_type: &str) -> bool {
        let normalized = article_type.trim().to_ascii_lowercase();
        match Self::from_article_type(&normalized) {
            Self::Narrative => true,
            Self::Academic => false,
            Self::Specialized => matches!(normalized.as_str(), "business" | "nonfiction"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_covers_all_supported_types() {
        // 与 skill_library::ARTICLE_TYPES 一一对应。
        assert_eq!(
            ArticlePolicy::from_article_type("academic"),
            ArticlePolicy::Academic
        );
        for ty in ["business", "legal", "tech_doc", "textbook", "nonfiction"] {
            assert_eq!(
                ArticlePolicy::from_article_type(ty),
                ArticlePolicy::Specialized,
                "type={ty}"
            );
        }
        for ty in ["fiction", "children", "blog", "news", "general"] {
            assert_eq!(
                ArticlePolicy::from_article_type(ty),
                ArticlePolicy::Narrative,
                "type={ty}"
            );
        }
    }

    #[test]
    fn unknown_type_falls_back_to_narrative() {
        assert_eq!(
            ArticlePolicy::from_article_type("unknown_thing"),
            ArticlePolicy::Narrative
        );
    }

    #[test]
    fn case_and_whitespace_are_normalized_once() {
        assert_eq!(
            ArticlePolicy::from_article_type("  Fiction "),
            ArticlePolicy::Narrative
        );
        assert_eq!(
            ArticlePolicy::from_article_type("ACADEMIC"),
            ArticlePolicy::Academic
        );
    }

    #[test]
    fn story_world_is_narrower_than_narrative() {
        assert!(ArticlePolicy::has_story_world("fiction"));
        assert!(ArticlePolicy::has_story_world("Children"));
        assert!(!ArticlePolicy::has_story_world("blog"));
        assert!(!ArticlePolicy::has_story_world("news"));
        assert!(!ArticlePolicy::has_story_world("general"));
        assert!(!ArticlePolicy::has_story_world("academic"));
    }

    #[test]
    fn person_term_acceptance_matches_semantics() {
        // narrative 全接受
        for ty in ["fiction", "children", "blog", "news", "general"] {
            assert!(ArticlePolicy::accepts_person_terms_for(ty), "type={ty}");
        }
        // specialized：business/nonfiction 接受，其余拒绝
        assert!(ArticlePolicy::accepts_person_terms_for("business"));
        assert!(ArticlePolicy::accepts_person_terms_for("nonfiction"));
        for ty in ["legal", "tech_doc", "textbook"] {
            assert!(!ArticlePolicy::accepts_person_terms_for(ty), "type={ty}");
        }
        // academic 拒绝
        assert!(!ArticlePolicy::accepts_person_terms_for("academic"));
        // 大小写不敏感（历史 matches! 精确匹配会拒绝 "Fiction"，集中化后统一）
        assert!(ArticlePolicy::accepts_person_terms_for("Fiction"));
    }

    #[test]
    fn specialized_or_academic_masks_specialized_spans() {
        assert!(ArticlePolicy::Academic.is_specialized_or_academic());
        assert!(ArticlePolicy::Specialized.is_specialized_or_academic());
        assert!(!ArticlePolicy::Narrative.is_specialized_or_academic());
    }
}
