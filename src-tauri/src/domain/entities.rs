use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    pub id: String,
    pub title: String,
    pub goal: String,
    pub system_prompt: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub archived_at: Option<i64>,
}

impl Workspace {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        goal: impl Into<String>,
        system_prompt: impl Into<String>,
        now: i64,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            goal: goal.into(),
            system_prompt: system_prompt.into(),
            created_at: now,
            updated_at: now,
            archived_at: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn {
    pub id: String,
    pub workspace_id: String,
    pub parent_run_id: Option<String>,
    pub prompt_markdown: String,
    pub title: Option<String>,
    pub created_at: i64,
}

impl Turn {
    pub fn root(
        id: impl Into<String>,
        workspace_id: impl Into<String>,
        prompt_markdown: impl Into<String>,
        created_at: i64,
    ) -> Self {
        Self {
            id: id.into(),
            workspace_id: workspace_id.into(),
            parent_run_id: None,
            prompt_markdown: prompt_markdown.into(),
            title: None,
            created_at,
        }
    }

    pub fn branch(
        id: impl Into<String>,
        workspace_id: impl Into<String>,
        parent_run_id: impl Into<String>,
        prompt_markdown: impl Into<String>,
        created_at: i64,
    ) -> Self {
        Self {
            id: id.into(),
            workspace_id: workspace_id.into(),
            parent_run_id: Some(parent_run_id.into()),
            prompt_markdown: prompt_markdown.into(),
            title: None,
            created_at,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentBlock {
    pub id: String,
    pub workspace_id: String,
    pub role: MessageRole,
    pub content: String,
    pub content_hash: String,
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderDialect {
    OpenAiCompatible,
    Ollama,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderProfile {
    pub id: String,
    pub name: String,
    pub dialect: ProviderDialect,
    pub base_url: String,
    pub model: String,
    pub parameters: BTreeMap<String, String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchPointer {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
    pub head_run_id: String,
    pub version: u64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionStatus {
    Adopted,
    Rejected,
    NeedsValidation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionMark {
    pub id: String,
    pub workspace_id: String,
    pub run_id: String,
    pub status: DecisionStatus,
    pub reason: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewState {
    pub workspace_id: String,
    pub view_key: String,
    pub state_json: String,
    pub updated_at: i64,
}
