pub mod en;
pub mod pt_br;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    En,
    PtBr,
}

impl Locale {
    pub fn from_env() -> Self {
        let locale_str = std::env::var("LOCALE")
            .unwrap_or_else(|_| "pt_br".to_string())
            .to_lowercase();

        match locale_str.as_str() {
            "en" | "english" => Locale::En,
            "pt_br" | "pt" | "portuguese" | _ => Locale::PtBr,
        }
    }
}

pub trait I18n {
    fn t(&self, key: MessageKey) -> &'static str;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKey {
    Help,
    HelpDescription,
    Clear,
    ClearDescription,
    Skills,
    SkillsDescription,
    Trust,
    TrustDescription,
    Summarize,
    SummarizeDescription,
    Stats,
    StatsDescription,
    Welcome,
    Error,
    ErrorPrefix,
    Success,
    SuccessPrefix,
    ToolExecuted,
    Observation,
    FinalAnswer,
    Thinking,
    Reasoning,
    Verification,
    Action,
    ActionInput,
    RetrievedMemory,
    ReviseMemory,
    CompressionsApplied,
    CurrentContextTokens,
    MaxContextTokens,
    ContextUsage,
    ApiCalls,
    Iterations,
    TotalTokens,
    PromptTokens,
    CompletionTokens,
    EstimatedCost,
    RateLimiterStatus,
    CallsRemaining,
    TokensRemaining,
    PerMinute,
    NotTrusted,
    Trusted,
    FullyTrusted,
    Untrusted,
    WorkspaceCurrent,
    WorkspaceTrust,
    WorkspacesConfigured,
    NoWorkspacesConfigured,
}

impl Locale {
    pub fn message(&self, key: MessageKey) -> &'static str {
        match self {
            Locale::En => en::message(key),
            Locale::PtBr => pt_br::message(key),
        }
    }
}

pub fn t(key: MessageKey) -> &'static str {
    Locale::from_env().message(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_locale_default_is_pt_br() {
        std::env::remove_var("LOCALE");
        let locale = Locale::from_env();
        assert_eq!(locale, Locale::PtBr);
    }

    #[test]
    fn test_locale_parsing() {
        assert_eq!(Locale::from_env(), Locale::PtBr);
    }
}
