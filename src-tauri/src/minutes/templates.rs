use super::{ContentType, MinutesError};

/// 标准会议模板 ID。
pub const STANDARD_MEETING_TEMPLATE_ID: &str = "standard_meeting";
/// 项目周会模板 ID。
pub const PROJECT_WEEKLY_TEMPLATE_ID: &str = "project_weekly";
/// 客户沟通模板 ID。
pub const CUSTOMER_COMMUNICATION_TEMPLATE_ID: &str = "customer_communication";
/// 课程总结模板 ID。
pub const COURSE_SUMMARY_TEMPLATE_ID: &str = "course_summary";
/// 课题研究模板 ID。
pub const RESEARCH_PROJECT_TEMPLATE_ID: &str = "research_project";
/// 学术讲座模板 ID。
pub const ACADEMIC_LECTURE_TEMPLATE_ID: &str = "academic_lecture";
/// 演讲总结模板 ID。
pub const SPEECH_SUMMARY_TEMPLATE_ID: &str = "speech_summary";
/// 人物专访模板 ID。
pub const PROFILE_INTERVIEW_TEMPLATE_ID: &str = "profile_interview";
/// 深度访谈模板 ID。
pub const IN_DEPTH_INTERVIEW_TEMPLATE_ID: &str = "in_depth_interview";
/// 商业计划书模板 ID。
pub const BUSINESS_PLAN_TEMPLATE_ID: &str = "business_plan";
/// 文章大纲模板 ID。
pub const ARTICLE_OUTLINE_TEMPLATE_ID: &str = "article_outline";
/// 自适应模板 ID。
pub const ADAPTIVE_TEMPLATE_ID: &str = "adaptive";
/// 内置模板首版的稳定版本。
pub const BUILTIN_TEMPLATE_VERSION: &str = "1.0.0";

/// 描述一份不可由 UI 改写的内置会议纪要模板。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinutesTemplate {
    pub id: &'static str,
    pub version: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub instructions: &'static str,
}

const STANDARD_MEETING: MinutesTemplate = MinutesTemplate {
    id: STANDARD_MEETING_TEMPLATE_ID,
    version: BUILTIN_TEMPLATE_VERSION,
    display_name: "标准会议纪要",
    description: "适用于日常讨论、部门例会与一般协作会议。",
    instructions: "平衡概括摘要、主要议题、结论、已确认决策、明确待办和风险问题；按讨论顺序组织议题。一般讨论、建议和设想不能升级为决策或待办。",
};

const PROJECT_WEEKLY: MinutesTemplate = MinutesTemplate {
    id: PROJECT_WEEKLY_TEMPLATE_ID,
    version: BUILTIN_TEMPLATE_VERSION,
    display_name: "项目周会",
    description: "适用于项目进展同步、任务协调与风险跟踪。",
    instructions: "议题优先按项目或工作流归类，摘要强调原文报告的进展，风险问题强调阻塞、依赖和偏差，待办只收明确后续动作。不得推算完成率；目标日期不自动成为待办截止日期。",
};

const CUSTOMER_COMMUNICATION: MinutesTemplate = MinutesTemplate {
    id: CUSTOMER_COMMUNICATION_TEMPLATE_ID,
    version: BUILTIN_TEMPLATE_VERSION,
    display_name: "客户沟通",
    description: "适用于需求沟通、方案演示与商务澄清。",
    instructions: "聚焦客户明确表达的诉求、澄清和约束；决策只收双方明确确认事项，待办只收明确对外或客户动作，异议和待确认内容归为问题。客户建议不等于我方承诺，不推断客户情绪、身份或合同义务。",
};

const COURSE_SUMMARY: MinutesTemplate = MinutesTemplate {
    id: COURSE_SUMMARY_TEMPLATE_ID,
    version: BUILTIN_TEMPLATE_VERSION,
    display_name: "课程总结",
    description: "适用于课堂、培训与系列课程的知识梳理。",
    instructions: "以学习者复习为目标，摘要概括课程主旨，主要议题按知识模块或教学顺序组织，关键结论提取定义、原理、方法和案例要点。只有讲者明确布置的练习或阅读才记为待办；不把教学示例误写为真实决策。",
};

const RESEARCH_PROJECT: MinutesTemplate = MinutesTemplate {
    id: RESEARCH_PROJECT_TEMPLATE_ID,
    version: BUILTIN_TEMPLATE_VERSION,
    display_name: "课题研究",
    description: "适用于课题研讨、研究进展汇报与方法论评审。",
    instructions: "围绕研究问题、背景、假设、方法、数据证据、阶段发现和局限组织议题。关键结论必须与转写中的证据强度一致，风险问题优先收录方法偏差、数据缺口和待验证假设。不把研究假设或相关性表述改写为已证实的因果结论。",
};

const ACADEMIC_LECTURE: MinutesTemplate = MinutesTemplate {
    id: ACADEMIC_LECTURE_TEMPLATE_ID,
    version: BUILTIN_TEMPLATE_VERSION,
    display_name: "学术讲座",
    description: "适用于学术报告、主题演讲与专家问答。",
    instructions: "contentType 使用 lecture。摘要突出讲座主旨与学术价值，主要议题按理论背景、核心论点、证据或案例、局限与问答组织。区分讲者主张、引用他人观点和听众提问；未得到回答的问题纳入风险和问题，不补写缺失的文献、数据或结论。",
};

const SPEECH_SUMMARY: MinutesTemplate = MinutesTemplate {
    id: SPEECH_SUMMARY_TEMPLATE_ID,
    version: BUILTIN_TEMPLATE_VERSION,
    display_name: "演讲总结",
    description: "适用于主题演讲、分享、致辞与单人观点陈述。",
    instructions: "contentType 使用 speech。以听众快速回顾为目标，按演讲推进顺序组织主题脉络、核心观点、论据、案例和启发。讲者的观点不能改写为会议共识、决策或团队待办；除非原文明示真实执行承诺，否则 decisions 和 actionItems 必须为空。",
};

const PROFILE_INTERVIEW: MinutesTemplate = MinutesTemplate {
    id: PROFILE_INTERVIEW_TEMPLATE_ID,
    version: BUILTIN_TEMPLATE_VERSION,
    display_name: "人物专访",
    description: "适用于人物故事、职业经历与观点型访谈。",
    instructions: "聚焦受访者的亲述经历、关键转折、价值观和代表性观点，主要议题可按时间线或人生主题组织。区分采访者问题与受访者回答；不根据语气推断性格、情绪或动机，不将个人回忆扩展为未核实的客观事实。",
};

const IN_DEPTH_INTERVIEW: MinutesTemplate = MinutesTemplate {
    id: IN_DEPTH_INTERVIEW_TEMPLATE_ID,
    version: BUILTIN_TEMPLATE_VERSION,
    display_name: "深度访谈",
    description: "适用于用户研究、定性调研与专业主题访谈。",
    instructions: "按研究主题归纳受访者的事实性描述、行为、需求、痛点、偏好和未解问题，保留重要条件与例外。关键结论只能是对本次访谈证据的概括；不将单一受访者观点泛化为整体人群结论，不把采访者的引导性提问当作受访者立场。",
};

const BUSINESS_PLAN: MinutesTemplate = MinutesTemplate {
    id: BUSINESS_PLAN_TEMPLATE_ID,
    version: BUILTIN_TEMPLATE_VERSION,
    display_name: "商业计划书",
    description: "适用于商业构想讨论、创业项目路演与计划评审。",
    instructions: "以问题与机会、目标客户、价值主张、产品或服务、市场与竞争、商业模式、执行路径、资源需求和风险假设为关注重点组织议题。决策和待办仅收录明确确认的行动；不自行生成市场数据、财务预测、竞争对手信息或融资条款。",
};

const ARTICLE_OUTLINE: MinutesTemplate = MinutesTemplate {
    id: ARTICLE_OUTLINE_TEMPLATE_ID,
    version: BUILTIN_TEMPLATE_VERSION,
    display_name: "文章大纲",
    description: "适用于将口述素材、选题讨论或内容会议整理成写作结构。",
    instructions: "将摘要作为文章主旨，将主要议题按逻辑递进关系组织为写作大纲，每个议题概括对应分节的核心论点、素材或案例。关键结论收录已明确的写作方向，待办只收明确约定的补充或编写任务；不补写转写中没有的论据或事实。",
};

const ADAPTIVE: MinutesTemplate = MinutesTemplate {
    id: ADAPTIVE_TEMPLATE_ID,
    version: BUILTIN_TEMPLATE_VERSION,
    display_name: "自适应模板",
    description: "由模型根据转写内容选择最合适的组织重点。",
    instructions: "先仅基于转写内容选择 contentType：多人协商、确认或任务协调才是 meeting；单人连续表达观点、主题分享或致辞是 speech；知识讲解按 lecture 或 course；问答主导是 interview；工作进展陈述但没有协商过程是 report；写作口述素材是 article_material；证据不足使用 other。不能因为出现‘我们’、多人声道或工作术语就判为会议。非会议内容按主题脉络组织 topics，把 conclusions 作为核心观点或知识点；不得把讲者观点、建议、案例和修辞句改成会议决策或团队待办，未出现明确执行承诺时 decisions 和 actionItems 必须为空。不得输出判断过程，不得编造转写和可信上下文中不存在的事实。",
};

const TEMPLATES: [MinutesTemplate; 12] = [
    STANDARD_MEETING,
    PROJECT_WEEKLY,
    CUSTOMER_COMMUNICATION,
    COURSE_SUMMARY,
    RESEARCH_PROJECT,
    ACADEMIC_LECTURE,
    SPEECH_SUMMARY,
    PROFILE_INTERVIEW,
    IN_DEPTH_INTERVIEW,
    BUSINESS_PLAN,
    ARTICLE_OUTLINE,
    ADAPTIVE,
];

/// 返回手动模板对应的可信内容类型；自适应模板由模型依据正文分类。
pub fn content_type_for_template(template_id: &str) -> Option<ContentType> {
    match template_id {
        STANDARD_MEETING_TEMPLATE_ID
        | PROJECT_WEEKLY_TEMPLATE_ID
        | CUSTOMER_COMMUNICATION_TEMPLATE_ID => Some(ContentType::Meeting),
        COURSE_SUMMARY_TEMPLATE_ID => Some(ContentType::Course),
        RESEARCH_PROJECT_TEMPLATE_ID | BUSINESS_PLAN_TEMPLATE_ID => Some(ContentType::Report),
        ACADEMIC_LECTURE_TEMPLATE_ID => Some(ContentType::Lecture),
        SPEECH_SUMMARY_TEMPLATE_ID => Some(ContentType::Speech),
        PROFILE_INTERVIEW_TEMPLATE_ID | IN_DEPTH_INTERVIEW_TEMPLATE_ID => {
            Some(ContentType::Interview)
        }
        ARTICLE_OUTLINE_TEMPLATE_ID => Some(ContentType::ArticleMaterial),
        ADAPTIVE_TEMPLATE_ID => None,
        _ => None,
    }
}

/// 返回稳定顺序的全部内置模板。
pub fn list_templates() -> &'static [MinutesTemplate] {
    &TEMPLATES
}

/// 按 ID 和版本解析一个内置模板。
pub fn get_template(
    template_id: &str,
    template_version: &str,
) -> Result<&'static MinutesTemplate, MinutesError> {
    let template = TEMPLATES
        .iter()
        .find(|template| template.id == template_id)
        .ok_or(MinutesError::UnknownTemplate)?;
    if template.version != template_version {
        return Err(MinutesError::TemplateVersionMismatch);
    }
    Ok(template)
}
