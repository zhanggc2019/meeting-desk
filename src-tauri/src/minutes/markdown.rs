use super::{ContentType, MeetingMinutes, RiskKind};

/// 返回内容类型对应的导出文案，避免把所有录音都写成会议。
fn content_headings(
    content_type: ContentType,
) -> (&'static str, &'static str, &'static str, &'static str) {
    match content_type {
        ContentType::Meeting => ("会议纪要", "会议摘要", "主要议题", "关键结论"),
        ContentType::Speech => ("演讲总结", "内容概览", "演讲脉络", "核心观点"),
        ContentType::Lecture => ("讲座总结", "内容概览", "讲座脉络", "核心观点"),
        ContentType::Course => ("课程总结", "课程概览", "知识脉络", "核心知识点"),
        ContentType::Interview => ("访谈总结", "访谈概览", "话题脉络", "关键洞察"),
        ContentType::Report => ("汇报总结", "汇报概览", "内容脉络", "关键结论"),
        ContentType::ArticleMaterial => ("口述素材整理", "内容概览", "章节结构", "核心观点"),
        ContentType::Other => ("录音内容整理", "内容概览", "主题脉络", "内容要点"),
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

/// 按固定章节顺序渲染已验证纪要；该接口不接受也不会输出完整 transcript。
pub fn render_minutes_markdown(minutes: &MeetingMinutes) -> String {
    let mut output = String::new();
    let (fallback_title, summary_heading, topics_heading, conclusions_heading) =
        content_headings(minutes.content_type);
    let title = minutes.title.as_deref().unwrap_or(fallback_title);
    output.push_str("# ");
    output.push_str(&escape_markdown(title));
    output.push_str("\n\n");

    output.push_str("- 内容类型：");
    output.push_str(content_type_label(minutes.content_type));
    if minutes.content_type == ContentType::Meeting {
        output.push_str("\n- 会议开始：");
        output.push_str(&render_optional(minutes.meeting_time.start_at.as_deref()));
        output.push_str("\n- 会议结束：");
        output.push_str(&render_optional(minutes.meeting_time.end_at.as_deref()));
        output.push_str("\n- 参会人：");
        if minutes.participants.is_empty() {
            output.push_str("未提供");
        } else {
            output.push_str(
                &minutes
                    .participants
                    .iter()
                    .map(|value| escape_markdown(value))
                    .collect::<Vec<_>>()
                    .join("、"),
            );
        }
    }
    output.push_str("\n\n## ");
    output.push_str(summary_heading);
    output.push_str("\n\n");
    output.push_str(&render_optional(minutes.summary.as_deref()));

    output.push_str("\n\n## ");
    output.push_str(topics_heading);
    output.push_str("\n\n");
    if minutes.topics.is_empty() {
        output.push_str("无明确议题");
    } else {
        for (index, topic) in minutes.topics.iter().enumerate() {
            output.push_str(&format!(
                "{}. **{}**",
                index + 1,
                escape_markdown(&topic.title)
            ));
            if let Some(summary) = topic.summary.as_deref() {
                output.push('：');
                output.push_str(&escape_markdown(summary));
            }
            output.push('\n');
        }
        output.pop();
    }

    render_statement_section(&mut output, conclusions_heading, &minutes.conclusions);
    if minutes.content_type == ContentType::Meeting || !minutes.decisions.is_empty() {
        render_statement_section(&mut output, "决策事项", &minutes.decisions);
    }

    if minutes.content_type == ContentType::Meeting || !minutes.action_items.is_empty() {
        output.push_str("\n\n## 待办事项\n\n");
        if minutes.action_items.is_empty() {
            output.push_str("无明确待办");
        } else {
            output.push_str("| 事项 | 负责人 | 截止日期 |\n| --- | --- | --- |\n");
            for item in &minutes.action_items {
                let due = item.due_date.as_deref().or(item.due_date_text.as_deref());
                output.push_str("| ");
                output.push_str(&escape_table_cell(&item.description));
                output.push_str(" | ");
                output.push_str(&escape_table_cell(
                    item.owner.as_deref().unwrap_or("未指定"),
                ));
                output.push_str(" | ");
                output.push_str(&escape_table_cell(due.unwrap_or("未指定")));
                output.push_str(" |\n");
            }
            output.pop();
        }
    }

    if minutes.content_type == ContentType::Meeting || !minutes.risks_and_issues.is_empty() {
        output.push_str("\n\n## 风险和问题\n\n");
        if minutes.risks_and_issues.is_empty() {
            output.push_str("无明确风险或问题");
        } else {
            for item in &minutes.risks_and_issues {
                let label = match item.kind {
                    RiskKind::Risk => "风险",
                    RiskKind::Issue => "问题",
                };
                output.push_str("- **");
                output.push_str(label);
                output.push_str("**：");
                output.push_str(&escape_markdown(&item.description));
                if let Some(impact) = item.impact.as_deref() {
                    output.push_str("；影响：");
                    output.push_str(&escape_markdown(impact));
                }
                if let Some(mitigation) = item.mitigation.as_deref() {
                    output.push_str("；原文措施：");
                    output.push_str(&escape_markdown(mitigation));
                }
                output.push('\n');
            }
            output.pop();
        }
    }

    output.push('\n');
    output
}

/// 渲染结论或决策列表章节。
fn render_statement_section(
    output: &mut String,
    heading: &str,
    statements: &[super::SupportedStatement],
) {
    output.push_str("\n\n## ");
    output.push_str(heading);
    output.push_str("\n\n");
    if statements.is_empty() {
        output.push_str("无明确内容");
        return;
    }
    for statement in statements {
        output.push_str("- ");
        output.push_str(&escape_markdown(&statement.content));
        output.push('\n');
    }
    output.pop();
}

/// 渲染 nullable 字段，不把 UI 占位回写为业务事实。
fn render_optional(value: Option<&str>) -> String {
    value
        .map(escape_markdown)
        .unwrap_or_else(|| "未提供".to_string())
}

/// 转义 Markdown 行内控制符、换行和 HTML 标签。
fn escape_markdown(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\\', "\\\\")
        .replace('`', "\\`")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace(['\r', '\n'], " ")
}

/// 转义 Markdown 表格单元格。
fn escape_table_cell(value: &str) -> String {
    escape_markdown(value).replace('|', "\\|")
}
