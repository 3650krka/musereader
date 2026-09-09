# 翻译引擎 Skill 系统

本模块采用类似 Claude Code Skill 的架构设计，将翻译提示组织为主体框架 + 辅助知识的模式。

## 架构设计

```
prompts/
├── SKILL.md                    # 核心翻译理论框架（主体）
├── __init__.py                 # Skill 加载模块
├── README.md                   # 本文档
└── references/                 # 类型专用规则（辅助知识）
    ├── academic.md             # 学术论文
    ├── fiction.md              # 小说
    ├── nonfiction.md           # 非虚构书籍
    ├── textbook.md             # 教材教科书
    ├── business.md             # 商业书籍
    ├── children.md             # 儿童读物
    ├── blog.md                 # 博客文章
    ├── news.md                 # 新闻报道
    ├── tech_doc.md             # 技术文档
    ├── legal.md                # 法律文本
    └── general.md              # 通用文章
```

## 核心理念

### SKILL.md - 核心翻译理论框架（主体）

包含多维翻译理论体系：
- **目的论 (Skopos Theory)**：定义翻译的全局目标
- **文本类型理论 (Text Typology)**：分析文本功能属性
- **关联理论 (Relevance Theory)**：处理认知负荷
- **多维翻译理论激活矩阵**：
  - 语义翻译 vs 交际翻译
  - 功能对等理论
  - 归化 vs 异化
  - 操控学派
- **底层语言学转换执行器**：增减译、词性转换、显隐性重组
- **执行流程**：从初始化到最终审视的完整工作流

### references/{type}.md - 类型专用规则（辅助知识注入）

针对特定文章类型的补充规则：
- 文本类型特征描述
- 特定的术语处理策略
- 格式保留要求
- 风格与语气指南

**重要**：类型专用规则是对核心理论框架的**补充和细化**，而非替代。

## 支持的文章类型

| 类型 | 代码 | 说明 |
|------|------|------|
| 学术论文 | `academic` | 默认类型，严谨的学术表达 |
| 小说 | `fiction` | 文学性强，注重叙事和艺术美感 |
| 非虚构书籍 | `nonfiction` | 传记、历史、科普等，注重事实准确性 |
| 教材教科书 | `textbook` | 结构化教学内容，术语规范化 |
| 商业书籍 | `business` | 管理、营销、创业等，实用性强 |
| 儿童读物 | `children` | 语言适龄化，富有童趣 |
| 博客文章 | `blog` | 口语化，个性化表达 |
| 新闻报道 | `news` | 客观中立，信息准确 |
| 技术文档 | `tech_doc` | 精确术语，代码保留 |
| 法律文本 | `legal` | 严谨准确，保守翻译 |
| 通用文章 | `general` | 平衡准确性和可读性 |

## 使用方式

### 命令行使用

```bash
# 默认（学术论文）
python -m backend.convert_pdf_to_zh_md document.pdf

# 指定文章类型
python -m backend.convert_pdf_to_zh_md novel.pdf --article-type fiction
python -m backend.convert_pdf_to_zh_md biography.pdf --article-type nonfiction
python -m backend.convert_pdf_to_zh_md textbook.pdf --article-type textbook
python -m backend.convert_pdf_to_zh_md business_book.pdf --article-type business
python -m backend.convert_pdf_to_zh_md children_book.pdf --article-type children

# 从已有 Markdown 翻译
python -m backend.convert_pdf_to_zh_md --source-markdown novel.md --article-type fiction
```

### Python API 使用

```python
from backend.markdown_translator import translate_markdown_document
from backend.sensenova_client import SensenovaClient

client = SensenovaClient(
    api_key="your_api_key",
    base_url="https://token.sensenova.cn/v1",
    model="sensenova-6.7-flash-lite",
)

# 翻译小说
result = translate_markdown_document(
    source_markdown,
    client=client,
    chunk_char_limit=8000,
    article_type="fiction",
)
```

## 提示加载机制

```python
# prompts/__init__.py
def load_prompts(article_type: str) -> TranslationPrompts:
    """
    加载翻译引擎提示
    
    架构：
    1. 读取 SKILL.md（核心翻译理论框架）
    2. 读取 references/{type}.md（类型专用规则）
    3. 组合：核心框架 + 类型规则（作为知识注入）
    """
    skill_content = _load_skill()
    reference_content = _load_reference(article_type)
    return _build_prompt(skill_content, reference_content)
```

### 提示结构

```
系统提示 = 核心翻译理论框架（主体）+ 类型专用规则（辅助知识注入）
```

**优势**：
- ✅ 核心理论框架统一，保证翻译质量的理论基础
- ✅ 类型专用规则作为补充，提供具体执行指导
- ✅ 清晰的主次关系，避免规则冲突
- ✅ 易于维护和扩展

## 添加新的文章类型

1. 在 `references/` 目录下创建新的 `{type}.md` 文件
2. 编写该类型的专用规则（作为对核心框架的补充）
3. 在 `__init__.py` 的 `AVAILABLE_TYPES` 字典中添加新类型
4. 测试新类型的翻译效果

## 与 Claude Code Skill 系统的对应关系

| Claude Code Skill | 翻译引擎 Skill | 说明 |
|-------------------|----------------|------|
| SKILL.md | SKILL.md | 核心原则和工作流 |
| references/ | references/ | 详细的参考文档 |
| scripts/ | （无） | 翻译引擎不需要脚本 |
| evals/ | （未实现） | 可添加翻译质量评估 |

## 设计原则

1. **核心框架为主**：SKILL.md 包含完整的翻译理论体系
2. **类型规则为辅**：references/{type}.md 提供具体执行指导
3. **知识注入模式**：类型规则补充核心框架，而非替代
4. **清晰的主次关系**：避免规则冲突和优先级混淆
5. **易于维护**：修改核心框架影响所有类型，修改类型规则只影响该类型

## 参考文档

- `SKILL.md` - 核心翻译理论框架
- `COORDINATION_OPTIMIZATION.md` - 协同机制优化方案（已过时，新架构已解决）
- `references/` - 各类型专用规则
