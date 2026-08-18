use std::fs::File;
use std::path::{Path, PathBuf};

use docx_rs::{
    Docx, Footer, LineSpacing, PageMargin, PageNum, Paragraph, Run, RunFonts, Shading, Table,
    TableCell, TableCellMargins, TableRow, WidthType,
};
use genpdf::{elements, fonts, style, Element as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::CommandError;
use crate::minutes::{ContentType, MeetingMinutes, RiskKind};

const WORD_TABLE_WIDTH: usize = 9_360;

/// 表示用户可选择的文档格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Docx,
    Pdf,
}

impl ExportFormat {
    /// 返回文档格式对应的安全文件扩展名。
    pub fn extension(self) -> &'static str {
        match self {
            Self::Docx => "docx",
            Self::Pdf => "pdf",
        }
    }
}

/// 表示用户可组合选择的导出内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportContent {
    Summary,
    Transcript,
    Minutes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExportDocument {
    title: String,
    subtitle: String,
    metadata: Vec<(String, String)>,
    sections: Vec<ExportSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExportSection {
    heading: String,
    blocks: Vec<ExportBlock>,
    page_break_before: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExportBlock {
    Paragraph(String),
    Entry {
        label: String,
        text: String,
    },
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

/// 按用户选择的格式和内容生成原子落盘的本地文档。
pub fn export_meeting_document(
    path: &Path,
    format: ExportFormat,
    contents: &[ExportContent],
    minutes: &MeetingMinutes,
    transcript: &Value,
) -> Result<(), CommandError> {
    let document = build_export_document(contents, minutes, transcript);
    let parent = path.parent().ok_or_else(|| {
        CommandError::new("export_path_invalid", "导出位置无效，请重新选择", false)
    })?;
    let staged = tempfile::Builder::new()
        .prefix("meeting-export-")
        .suffix(&format!(".{}", format.extension()))
        .tempfile_in(parent)
        .map_err(|_| CommandError::new("export_failed", "无法创建导出文件", true))?;
    match format {
        ExportFormat::Docx => render_docx(staged.path(), &document)?,
        ExportFormat::Pdf => render_pdf(staged.path(), &document)?,
    }
    staged
        .as_file()
        .sync_all()
        .map_err(|_| CommandError::new("export_failed", "无法完整写入导出文件", true))?;
    staged
        .persist(path)
        .map_err(|_| CommandError::new("export_failed", "无法保存到所选位置", true))?;
    Ok(())
}

/// 把结构化纪要和逐字稿整理成格式无关的文档模型。
fn build_export_document(
    contents: &[ExportContent],
    minutes: &MeetingMinutes,
    transcript: &Value,
) -> ExportDocument {
    let (subtitle, summary_heading, topics_heading, conclusions_heading) =
        content_headings(minutes.content_type);
    let mut metadata = vec![(
        "内容类型".to_string(),
        content_type_label(minutes.content_type).to_string(),
    )];
    if minutes.content_type == ContentType::Meeting {
        if let Some(start_at) = minutes.meeting_time.start_at.as_deref() {
            metadata.push(("开始时间".to_string(), start_at.to_string()));
        }
        if !minutes.participants.is_empty() {
            metadata.push(("参会人".to_string(), minutes.participants.join("、")));
        }
    }

    let mut sections = Vec::new();
    if contents.contains(&ExportContent::Summary) {
        sections.push(ExportSection {
            heading: summary_heading.to_string(),
            blocks: vec![ExportBlock::Paragraph(
                minutes
                    .summary
                    .clone()
                    .unwrap_or_else(|| "未提取到摘要".to_string()),
            )],
            page_break_before: false,
        });
    }
    if contents.contains(&ExportContent::Minutes) {
        append_minutes_sections(&mut sections, minutes, topics_heading, conclusions_heading);
    }
    if contents.contains(&ExportContent::Transcript) {
        sections.push(ExportSection {
            heading: "逐字稿".to_string(),
            blocks: transcript_blocks(transcript),
            page_break_before: !sections.is_empty(),
        });
    }

    ExportDocument {
        title: minutes
            .title
            .clone()
            .unwrap_or_else(|| subtitle.to_string()),
        subtitle: subtitle.to_string(),
        metadata,
        sections,
    }
}

/// 把 AI 纪要中的结构化字段追加为独立章节，避免与单独摘要重复。
fn append_minutes_sections(
    sections: &mut Vec<ExportSection>,
    minutes: &MeetingMinutes,
    topics_heading: &str,
    conclusions_heading: &str,
) {
    sections.push(ExportSection {
        heading: topics_heading.to_string(),
        blocks: if minutes.topics.is_empty() {
            vec![ExportBlock::Paragraph("未提取到主题内容".to_string())]
        } else {
            minutes
                .topics
                .iter()
                .enumerate()
                .map(|(index, topic)| ExportBlock::Entry {
                    label: format!("主题 {:02}", index + 1),
                    text: topic
                        .summary
                        .as_deref()
                        .map(|summary| format!("{}\n{}", topic.title, summary))
                        .unwrap_or_else(|| topic.title.clone()),
                })
                .collect()
        },
        page_break_before: false,
    });
    sections.push(statement_section(conclusions_heading, &minutes.conclusions));
    if minutes.content_type == ContentType::Meeting || !minutes.decisions.is_empty() {
        sections.push(statement_section("决策事项", &minutes.decisions));
    }
    if minutes.content_type == ContentType::Meeting || !minutes.action_items.is_empty() {
        let rows = minutes
            .action_items
            .iter()
            .map(|item| {
                vec![
                    item.description.clone(),
                    item.owner.clone().unwrap_or_else(|| "未指定".to_string()),
                    item.due_date
                        .clone()
                        .or_else(|| item.due_date_text.clone())
                        .unwrap_or_else(|| "未指定".to_string()),
                ]
            })
            .collect::<Vec<_>>();
        sections.push(ExportSection {
            heading: "待办事项".to_string(),
            blocks: if rows.is_empty() {
                vec![ExportBlock::Paragraph("无明确待办".to_string())]
            } else {
                vec![ExportBlock::Table {
                    headers: vec![
                        "事项".to_string(),
                        "负责人".to_string(),
                        "截止日期".to_string(),
                    ],
                    rows,
                }]
            },
            page_break_before: false,
        });
    }
    if minutes.content_type == ContentType::Meeting || !minutes.risks_and_issues.is_empty() {
        let blocks = if minutes.risks_and_issues.is_empty() {
            vec![ExportBlock::Paragraph("无明确风险或问题".to_string())]
        } else {
            minutes
                .risks_and_issues
                .iter()
                .map(|item| {
                    let mut text = item.description.clone();
                    if let Some(impact) = item.impact.as_deref() {
                        text.push_str("\n影响：");
                        text.push_str(impact);
                    }
                    if let Some(mitigation) = item.mitigation.as_deref() {
                        text.push_str("\n应对：");
                        text.push_str(mitigation);
                    }
                    ExportBlock::Entry {
                        label: match item.kind {
                            RiskKind::Risk => "风险".to_string(),
                            RiskKind::Issue => "问题".to_string(),
                        },
                        text,
                    }
                })
                .collect()
        };
        sections.push(ExportSection {
            heading: "风险和问题".to_string(),
            blocks,
            page_break_before: false,
        });
    }
}

/// 把结论或决策数组转换为带稳定标签的文档章节。
fn statement_section(
    heading: &str,
    statements: &[crate::minutes::SupportedStatement],
) -> ExportSection {
    ExportSection {
        heading: heading.to_string(),
        blocks: if statements.is_empty() {
            vec![ExportBlock::Paragraph("无明确内容".to_string())]
        } else {
            statements
                .iter()
                .map(|statement| ExportBlock::Entry {
                    label: heading.trim_end_matches("事项").to_string(),
                    text: statement.content.clone(),
                })
                .collect()
        },
        page_break_before: false,
    }
}

/// 优先使用带时间戳和说话人的分段构建逐字稿，否则回退到正文段落。
fn transcript_blocks(transcript: &Value) -> Vec<ExportBlock> {
    let segments = transcript["segments"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if !segments.is_empty() {
        return segments
            .into_iter()
            .filter_map(|segment| {
                let text = segment["text"].as_str()?.trim();
                if text.is_empty() {
                    return None;
                }
                let speaker = segment["speakerLabel"].as_str().unwrap_or("录音内容");
                let label = segment["startMs"]
                    .as_u64()
                    .map(|milliseconds| format!("{}  {}", format_timestamp(milliseconds), speaker))
                    .unwrap_or_else(|| speaker.to_string());
                Some(ExportBlock::Entry {
                    label,
                    text: text.to_string(),
                })
            })
            .collect();
    }
    let text = transcript["text"].as_str().unwrap_or_default();
    let paragraphs = split_plain_transcript(text);
    if paragraphs.is_empty() {
        vec![ExportBlock::Paragraph("无可用逐字稿".to_string())]
    } else {
        paragraphs.into_iter().map(ExportBlock::Paragraph).collect()
    }
}

/// 把没有结构化分段的旧逐字稿按换行和目标长度整理成可读段落。
fn split_plain_transcript(text: &str) -> Vec<String> {
    const TARGET_CHARS: usize = 180;
    let mut paragraphs = Vec::new();
    for source_line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let mut paragraph = String::new();
        for character in source_line.chars() {
            paragraph.push(character);
            if paragraph.chars().count() >= TARGET_CHARS
                && matches!(character, '。' | '！' | '？' | '；' | '.' | '!' | '?')
            {
                paragraphs.push(std::mem::take(&mut paragraph));
            }
        }
        if !paragraph.is_empty() {
            paragraphs.push(paragraph);
        }
    }
    paragraphs
}

/// 把毫秒时间戳格式化为逐字稿使用的时分秒。
fn format_timestamp(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    let hours = seconds / 3_600;
    let minutes = (seconds % 3_600) / 60;
    let seconds = seconds % 60;
    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

/// 返回内容类型对应的文档标题与章节文案。
fn content_headings(
    content_type: ContentType,
) -> (&'static str, &'static str, &'static str, &'static str) {
    match content_type {
        ContentType::Meeting => ("AI 会议纪要", "会议摘要", "主要议题", "关键结论"),
        ContentType::Speech => ("演讲内容整理", "内容摘要", "演讲脉络", "核心观点"),
        ContentType::Lecture => ("讲座内容整理", "内容摘要", "讲座脉络", "核心观点"),
        ContentType::Course => ("课程内容整理", "课程摘要", "知识脉络", "核心知识点"),
        ContentType::Interview => ("访谈内容整理", "访谈摘要", "话题脉络", "关键洞察"),
        ContentType::Report => ("汇报内容整理", "汇报摘要", "内容脉络", "关键结论"),
        ContentType::ArticleMaterial => ("口述素材整理", "内容摘要", "章节结构", "核心观点"),
        ContentType::Other => ("录音内容整理", "内容摘要", "主题脉络", "内容要点"),
    }
}

/// 返回内容类型的稳定中文名称。
fn content_type_label(content_type: ContentType) -> &'static str {
    match content_type {
        ContentType::Meeting => "会议",
        ContentType::Speech => "演讲",
        ContentType::Lecture => "讲座",
        ContentType::Course => "课程",
        ContentType::Interview => "访谈",
        ContentType::Report => "汇报",
        ContentType::ArticleMaterial => "口述素材",
        ContentType::Other => "其他内容",
    }
}

/// 使用标准业务简报版式生成 Word 文档。
fn render_docx(path: &Path, document: &ExportDocument) -> Result<(), CommandError> {
    let fonts = word_fonts();
    let footer = Footer::new().add_paragraph(
        Paragraph::new()
            .align(docx_rs::AlignmentType::Right)
            .add_run(word_run("听见纪要  |  ", 18, false, "7A8793"))
            .add_page_num(PageNum::new()),
    );
    let mut docx = Docx::new()
        .page_size(12_240, 15_840)
        .page_margin(
            PageMargin::new()
                .top(1_440)
                .right(1_440)
                .bottom(1_440)
                .left(1_440)
                .header(720)
                .footer(720),
        )
        .default_fonts(fonts)
        .default_size(22)
        .default_line_spacing(LineSpacing::new().line(276).after(120))
        .footer(footer)
        .add_paragraph(
            Paragraph::new()
                .line_spacing(LineSpacing::new().after(80))
                .add_run(word_run("听见纪要 · 本地导出", 19, true, "52708F")),
        )
        .add_paragraph(
            Paragraph::new()
                .keep_next(true)
                .line_spacing(LineSpacing::new().after(80))
                .add_run(word_run(&document.title, 48, true, "1F3347")),
        )
        .add_paragraph(
            Paragraph::new()
                .line_spacing(LineSpacing::new().after(220))
                .add_run(word_run(&document.subtitle, 24, false, "657789")),
        );

    for (label, value) in &document.metadata {
        docx = docx.add_paragraph(
            Paragraph::new()
                .line_spacing(LineSpacing::new().after(55))
                .add_run(word_run(&format!("{label}："), 20, true, "465D72"))
                .add_run(word_run(value, 20, false, "536271")),
        );
    }

    for section in &document.sections {
        docx = docx.add_paragraph(
            Paragraph::new()
                .page_break_before(section.page_break_before)
                .keep_next(true)
                .line_spacing(LineSpacing::new().before(320).after(150))
                .add_run(word_run(&section.heading, 32, true, "2E5C8A")),
        );
        for block in &section.blocks {
            docx = match block {
                ExportBlock::Paragraph(text) => add_word_paragraphs(docx, text),
                ExportBlock::Entry { label, text } => add_word_entry(docx, label, text),
                ExportBlock::Table { headers, rows } => {
                    docx.add_table(build_word_table(headers, rows))
                }
            };
        }
    }

    let file = File::create(path)
        .map_err(|_| CommandError::new("export_failed", "无法创建 Word 文档", true))?;
    docx.build()
        .pack(file)
        .map_err(|_| CommandError::new("export_failed", "Word 文档生成失败", true))?;
    Ok(())
}

/// 返回同时覆盖拉丁字符和中文的 Word 字体配置。
fn word_fonts() -> RunFonts {
    RunFonts::new()
        .ascii("Aptos")
        .hi_ansi("Aptos")
        .east_asia("Microsoft YaHei")
        .cs("Aptos")
}

/// 创建带统一字体、字号和颜色的 Word 文本运行。
fn word_run(text: &str, size: usize, bold: bool, color: &str) -> Run {
    let run = Run::new()
        .add_text(text)
        .size(size)
        .color(color)
        .fonts(word_fonts());
    if bold {
        run.bold()
    } else {
        run
    }
}

/// 按原有换行把正文加入 Word，防止换行被折叠。
fn add_word_paragraphs(mut docx: Docx, text: &str) -> Docx {
    let lines = text.lines().collect::<Vec<_>>();
    for line in if lines.is_empty() { vec![""] } else { lines } {
        docx = docx.add_paragraph(
            Paragraph::new()
                .line_spacing(LineSpacing::new().after(120).line(300))
                .add_run(word_run(line, 22, false, "334556")),
        );
    }
    docx
}

/// 把带标签的内容渲染为 Word 的紧凑事实块。
fn add_word_entry(mut docx: Docx, label: &str, text: &str) -> Docx {
    docx = docx.add_paragraph(
        Paragraph::new()
            .keep_next(true)
            .line_spacing(LineSpacing::new().before(100).after(45))
            .add_run(word_run(label, 19, true, "52708F")),
    );
    add_word_paragraphs(docx, text)
}

/// 构造固定列宽和单元格内边距的 Word 数据表。
fn build_word_table(headers: &[String], rows: &[Vec<String>]) -> Table {
    let column_count = headers.len().max(1);
    let widths = if column_count == 3 {
        vec![5_200, 1_800, 2_360]
    } else {
        vec![WORD_TABLE_WIDTH / column_count; column_count]
    };
    let header_cells = headers
        .iter()
        .zip(widths.iter())
        .map(|(header, width)| {
            TableCell::new()
                .width(*width, WidthType::Dxa)
                .shading(Shading::new().fill("E8EEF5"))
                .add_paragraph(
                    Paragraph::new()
                        .line_spacing(LineSpacing::new().after(0))
                        .add_run(word_run(header, 20, true, "40566B")),
                )
        })
        .collect();
    let mut table_rows = vec![TableRow::new(header_cells).cant_split()];
    for row in rows {
        let cells = row
            .iter()
            .zip(widths.iter())
            .map(|(value, width)| {
                TableCell::new()
                    .width(*width, WidthType::Dxa)
                    .vertical_align(docx_rs::VAlignType::Center)
                    .add_paragraph(
                        Paragraph::new()
                            .line_spacing(LineSpacing::new().after(0).line(276))
                            .add_run(word_run(value, 20, false, "334556")),
                    )
            })
            .collect();
        table_rows.push(TableRow::new(cells).cant_split());
    }
    Table::new(table_rows)
        .set_grid(widths)
        .width(WORD_TABLE_WIDTH, WidthType::Dxa)
        .indent(120)
        .margins(TableCellMargins::new().margin(90, 120, 90, 120))
}

/// 使用 Windows 自带中文字体生成可复制、可搜索的 PDF。
fn render_pdf(path: &Path, document: &ExportDocument) -> Result<(), CommandError> {
    let family = load_windows_pdf_fonts()?;
    let mut pdf = genpdf::Document::new(family);
    pdf.set_title(document.title.clone());
    pdf.set_paper_size(genpdf::PaperSize::Letter);
    pdf.set_font_size(11);
    pdf.set_line_spacing(1.25);
    pdf.set_minimal_conformance();
    let mut decorator = genpdf::SimplePageDecorator::new();
    decorator.set_margins(genpdf::Margins::all(25.4));
    decorator.set_header(|page| {
        pdf_paragraph(
            &format!("听见纪要  |  第 {page} 页"),
            style::Style::new()
                .with_font_size(9)
                .with_color(style::Color::Rgb(112, 128, 143)),
        )
        .aligned(genpdf::Alignment::Right)
        .padded(genpdf::Margins::trbl(0, 0, 4, 0))
    });
    pdf.set_page_decorator(decorator);

    pdf.push(pdf_paragraph(
        "听见纪要 · 本地导出",
        style::Style::new()
            .with_font_size(10)
            .with_color(style::Color::Rgb(82, 112, 143)),
    ));
    pdf.push(elements::Break::new(0.4));
    pdf.push(pdf_paragraph(
        &document.title,
        style::Style::new()
            .with_font_size(23)
            .with_color(style::Color::Rgb(31, 51, 71)),
    ));
    pdf.push(elements::Break::new(0.45));
    pdf.push(pdf_paragraph(
        &document.subtitle,
        style::Style::new()
            .with_font_size(12)
            .with_color(style::Color::Rgb(101, 119, 137)),
    ));
    pdf.push(elements::Break::new(0.8));
    for (label, value) in &document.metadata {
        let mut paragraph = pdf_paragraph(
            &format!("{label}："),
            style::Style::new()
                .with_font_size(10)
                .with_color(style::Color::Rgb(70, 93, 114)),
        );
        append_pdf_tokens(
            &mut paragraph,
            value,
            style::Style::new()
                .with_font_size(10)
                .with_color(style::Color::Rgb(83, 98, 113)),
        );
        pdf.push(paragraph);
        pdf.push(elements::Break::new(0.2));
    }

    for section in &document.sections {
        if section.page_break_before {
            pdf.push(elements::PageBreak::new());
        } else {
            pdf.push(elements::Break::new(0.9));
        }
        pdf.push(pdf_paragraph(
            &section.heading,
            style::Style::new()
                .with_font_size(16)
                .with_color(style::Color::Rgb(46, 92, 138)),
        ));
        pdf.push(elements::Break::new(0.45));
        for block in &section.blocks {
            match block {
                ExportBlock::Paragraph(text) => push_pdf_text(&mut pdf, text),
                ExportBlock::Entry { label, text } => push_pdf_entry(&mut pdf, label, text),
                ExportBlock::Table { headers, rows } => {
                    pdf.push(build_pdf_table(headers, rows)?);
                    pdf.push(elements::Break::new(0.4));
                }
            }
        }
    }
    pdf.render_to_file(path)
        .map_err(|_| CommandError::new("export_failed", "PDF 文档生成失败", true))?;
    Ok(())
}

/// 从 Windows 字体目录加载 PDF 使用的常规和粗体中文字体。
fn load_windows_pdf_fonts() -> Result<fonts::FontFamily<fonts::FontData>, CommandError> {
    let windows_dir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .ok_or_else(|| CommandError::new("pdf_font_missing", "未找到 Windows 字体目录", false))?;
    let font_dir = windows_dir.join("Fonts");
    let regular_path = first_existing_path(&font_dir, &["simhei.ttf", "Deng.ttf"])
        .ok_or_else(|| CommandError::new("pdf_font_missing", "未找到可用的中文字体", false))?;
    let latin_regular_path = first_existing_path(&font_dir, &["arial.ttf"])
        .ok_or_else(|| CommandError::new("pdf_font_missing", "未找到可用的西文字体", false))?;
    let latin_bold_path = first_existing_path(&font_dir, &["arialbd.ttf", "arial.ttf"])
        .unwrap_or_else(|| latin_regular_path.clone());
    let latin_italic_path = first_existing_path(&font_dir, &["ariali.ttf", "arial.ttf"])
        .unwrap_or_else(|| latin_regular_path.clone());
    let latin_bold_italic_path = first_existing_path(&font_dir, &["arialbi.ttf", "arialbd.ttf"])
        .unwrap_or_else(|| latin_bold_path.clone());
    let regular = fonts::FontData::load(&regular_path, None)
        .map_err(|_| CommandError::new("pdf_font_invalid", "Windows 中文字体无法加载", false))?;
    let bold = fonts::FontData::load(&latin_bold_path, Some(printpdf::BuiltinFont::HelveticaBold))
        .map_err(|_| CommandError::new("pdf_font_invalid", "Windows 字体无法加载", false))?;
    let italic = fonts::FontData::load(
        &latin_italic_path,
        Some(printpdf::BuiltinFont::HelveticaOblique),
    )
    .map_err(|_| CommandError::new("pdf_font_invalid", "Windows 字体无法加载", false))?;
    let bold_italic = fonts::FontData::load(
        &latin_bold_italic_path,
        Some(printpdf::BuiltinFont::HelveticaBoldOblique),
    )
    .map_err(|_| CommandError::new("pdf_font_invalid", "Windows 字体无法加载", false))?;
    Ok(fonts::FontFamily {
        regular,
        bold,
        italic,
        bold_italic,
    })
}

/// 在候选字体名称中返回第一个存在的文件路径。
fn first_existing_path(directory: &Path, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .map(|name| directory.join(name))
        .find(|path| path.is_file())
}

/// 把中文字符和拉丁词拆成可安全换行的 PDF 文本标记。
fn pdf_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut latin = String::new();
    for character in text.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '/' | '.' | ':') {
            latin.push(character);
            continue;
        }
        if !latin.is_empty() {
            tokens.push(std::mem::take(&mut latin));
        }
        tokens.push(character.to_string());
    }
    if !latin.is_empty() {
        tokens.push(latin);
    }
    tokens
}

/// 创建支持中文逐字换行的 PDF 段落。
fn pdf_paragraph(text: &str, text_style: style::Style) -> elements::Paragraph {
    let mut paragraph = elements::Paragraph::default();
    append_pdf_tokens(&mut paragraph, text, text_style);
    paragraph
}

/// 向现有 PDF 段落追加一组可换行的中文文本标记。
fn append_pdf_tokens(paragraph: &mut elements::Paragraph, text: &str, text_style: style::Style) {
    for token in pdf_tokens(text) {
        paragraph.push_styled(token, text_style);
    }
}

/// 按原有换行把普通正文加入 PDF。
fn push_pdf_text(pdf: &mut genpdf::Document, text: &str) {
    for line in text.lines() {
        pdf.push(pdf_paragraph(
            line,
            style::Style::new()
                .with_font_size(11)
                .with_color(style::Color::Rgb(51, 69, 86)),
        ));
        pdf.push(elements::Break::new(0.35));
    }
}

/// 把带标签的事实块加入 PDF。
fn push_pdf_entry(pdf: &mut genpdf::Document, label: &str, text: &str) {
    pdf.push(pdf_paragraph(
        label,
        style::Style::new()
            .with_font_size(10)
            .with_color(style::Color::Rgb(82, 112, 143)),
    ));
    pdf.push(elements::Break::new(0.15));
    push_pdf_text(pdf, text);
}

/// 构造可跨页换行且带边框的 PDF 数据表。
fn build_pdf_table(
    headers: &[String],
    rows: &[Vec<String>],
) -> Result<elements::TableLayout, CommandError> {
    let weights = if headers.len() == 3 {
        vec![5, 2, 2]
    } else {
        vec![1; headers.len()]
    };
    let mut table = elements::TableLayout::new(weights);
    table.set_cell_decorator(elements::FrameCellDecorator::new(true, true, false));
    let mut header_row = table.row();
    for header in headers {
        header_row.push_element(
            pdf_paragraph(
                header,
                style::Style::new()
                    .with_font_size(9)
                    .with_color(style::Color::Rgb(64, 86, 107)),
            )
            .padded(1.5),
        );
    }
    header_row
        .push()
        .map_err(|_| CommandError::new("export_failed", "PDF 表格结构无效", false))?;
    for row in rows {
        let mut table_row = table.row();
        for value in row {
            table_row.push_element(
                pdf_paragraph(
                    value,
                    style::Style::new()
                        .with_font_size(9)
                        .with_color(style::Color::Rgb(51, 69, 86)),
                )
                .padded(1.5),
            );
        }
        table_row
            .push()
            .map_err(|_| CommandError::new("export_failed", "PDF 表格结构无效", false))?;
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::minutes::{MeetingTime, TitleSource, MEETING_MINUTES_SCHEMA_VERSION};

    /// 构造同时覆盖摘要、纪要表格和逐字稿的测试数据。
    fn fixture_minutes() -> MeetingMinutes {
        MeetingMinutes {
            schema_version: MEETING_MINUTES_SCHEMA_VERSION.to_string(),
            content_type: ContentType::Meeting,
            title: Some("产品交付讨论".to_string()),
            title_source: TitleSource::Generated,
            meeting_time: MeetingTime {
                start_at: Some("2026-08-17T09:00:00+08:00".to_string()),
                end_at: None,
            },
            participants: vec!["产品组".to_string(), "研发组".to_string()],
            summary: Some("团队确认先完成导出和应用内试听。".to_string()),
            topics: vec![crate::minutes::Topic {
                title: "交付范围".to_string(),
                summary: Some("明确 Word、PDF 和播放器范围。".to_string()),
                evidence_segment_ids: vec!["segment-1".to_string()],
            }],
            conclusions: vec![crate::minutes::SupportedStatement {
                content: "导出内容由用户自由组合。".to_string(),
                evidence_segment_ids: vec!["segment-1".to_string()],
            }],
            decisions: Vec::new(),
            action_items: vec![crate::minutes::ActionItem {
                description: "完成导出验证".to_string(),
                owner: Some("研发组".to_string()),
                due_date_text: Some("本周".to_string()),
                due_date: None,
                evidence_segment_ids: vec!["segment-1".to_string()],
            }],
            risks_and_issues: Vec::new(),
        }
    }

    /// 验证导出内容选择会生成互不混淆的章节。
    #[test]
    fn builds_selected_document_sections() {
        let transcript = serde_json::json!({
            "text": "第一段",
            "segments": [{"id": "segment-1", "startMs": 5_000, "speakerLabel": "说话人 A", "text": "第一段"}]
        });
        let document = build_export_document(
            &[ExportContent::Summary, ExportContent::Transcript],
            &fixture_minutes(),
            &transcript,
        );
        assert_eq!(document.sections.len(), 2);
        assert_eq!(document.sections[0].heading, "会议摘要");
        assert_eq!(document.sections[1].heading, "逐字稿");
        assert!(document.sections[1].page_break_before);
        assert!(matches!(
            &document.sections[1].blocks[0],
            ExportBlock::Entry { label, .. } if label.contains("00:05")
        ));

        let transcript_only = build_export_document(
            &[ExportContent::Transcript],
            &fixture_minutes(),
            &transcript,
        );
        assert!(!transcript_only.sections[0].page_break_before);
    }

    /// 验证 Word 与 PDF 都能生成非空的真实文档文件。
    #[test]
    fn exports_docx_and_pdf_files() {
        let directory = tempfile::TempDir::new().expect("create export directory");
        let output_directory = std::env::var_os("MEETING_EXPORT_QA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| directory.path().to_path_buf());
        std::fs::create_dir_all(&output_directory).expect("create configured export directory");
        let transcript = serde_json::json!({
            "text": "第一段。第二段。",
            "segments": [{"id": "segment-1", "startMs": 5_000, "speakerLabel": "说话人 A", "text": "第一段。"}]
        });
        let contents = [
            ExportContent::Summary,
            ExportContent::Minutes,
            ExportContent::Transcript,
        ];
        for format in [ExportFormat::Docx, ExportFormat::Pdf] {
            let path = output_directory.join(format!("sample.{}", format.extension()));
            export_meeting_document(&path, format, &contents, &fixture_minutes(), &transcript)
                .expect("export document");
            assert!(std::fs::metadata(path).expect("read export metadata").len() > 1_000);
        }
    }

    /// 验证中文和英文会拆成 PDF 可换行标记且不丢失原文。
    #[test]
    fn tokenizes_pdf_text_without_losing_content() {
        let source = "中文ASR-v2.0 测试";
        assert_eq!(pdf_tokens(source).concat(), source);
    }
}
