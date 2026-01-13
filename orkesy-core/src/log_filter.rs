#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Debug => write!(f, "DEBUG"),
            Self::Info => write!(f, "INFO"),
            Self::Warn => write!(f, "WARN"),
            Self::Error => write!(f, "ERROR"),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LogFilterMode {
    #[default]
    All,
    WarnAndAbove,
    ErrorOnly,
}

impl LogFilterMode {
    pub fn cycle(self) -> Self {
        match self {
            Self::All => Self::WarnAndAbove,
            Self::WarnAndAbove => Self::ErrorOnly,
            Self::ErrorOnly => Self::All,
        }
    }

    pub fn matches(&self, level: LogLevel) -> bool {
        match self {
            Self::All => true,
            Self::WarnAndAbove => matches!(level, LogLevel::Warn | LogLevel::Error),
            Self::ErrorOnly => matches!(level, LogLevel::Error),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::All => "ALL",
            Self::WarnAndAbove => "WARN+",
            Self::ErrorOnly => "ERROR",
        }
    }
}

pub fn detect_level(text: &str) -> LogLevel {
    let lower = text.to_lowercase();

    // Check for error indicators
    if lower.contains("error")
        || lower.contains("[err]")
        || lower.contains("[error]")
        || lower.contains("fatal")
        || lower.contains("panic")
        || lower.contains("exception")
        || lower.starts_with("e ")
    {
        return LogLevel::Error;
    }

    // Check for warning indicators
    if lower.contains("warn")
        || lower.contains("[wrn]")
        || lower.contains("[warning]")
        || lower.contains("deprecat")
        || lower.starts_with("w ")
    {
        return LogLevel::Warn;
    }

    // Check for debug indicators
    if lower.contains("debug")
        || lower.contains("[dbg]")
        || lower.contains("[debug]")
        || lower.contains("trace")
        || lower.starts_with("d ")
    {
        return LogLevel::Debug;
    }

    // Default to info
    LogLevel::Info
}

#[derive(Clone, Debug, Default)]
pub struct GrepFilter {
    pub pattern: Option<String>,
    pub case_sensitive: bool,
}

impl GrepFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_pattern(pattern: String) -> Self {
        Self {
            pattern: Some(pattern),
            case_sensitive: false,
        }
    }

    pub fn with_case_sensitive(mut self, case_sensitive: bool) -> Self {
        self.case_sensitive = case_sensitive;
        self
    }

    pub fn matches(&self, text: &str) -> bool {
        match &self.pattern {
            None => true,
            Some(p) if p.is_empty() => true,
            Some(p) if self.case_sensitive => text.contains(p),
            Some(p) => text.to_lowercase().contains(&p.to_lowercase()),
        }
    }

    pub fn clear(&mut self) {
        self.pattern = None;
    }

    pub fn set(&mut self, pattern: String) {
        self.pattern = if pattern.is_empty() {
            None
        } else {
            Some(pattern)
        };
    }
}

#[derive(Clone, Debug, Default)]
pub struct LogFilter {
    pub level_mode: LogFilterMode,
    pub grep: GrepFilter,
}

impl LogFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn should_show(&self, text: &str) -> bool {
        let level = detect_level(text);
        self.level_mode.matches(level) && self.grep.matches(text)
    }

    pub fn cycle_level(&mut self) {
        self.level_mode = self.level_mode.cycle();
    }

    pub fn label(&self) -> String {
        match &self.grep.pattern {
            Some(p) if !p.is_empty() => format!("{} /{}/", self.level_mode.label(), p),
            _ => self.level_mode.label().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_level() {
        assert_eq!(detect_level("ERROR: something failed"), LogLevel::Error);
        assert_eq!(detect_level("[error] connection failed"), LogLevel::Error);
        assert_eq!(detect_level("WARN: deprecated API"), LogLevel::Warn);
        assert_eq!(detect_level("[warning] slow query"), LogLevel::Warn);
        assert_eq!(detect_level("DEBUG: variable x = 5"), LogLevel::Debug);
        assert_eq!(detect_level("GET /health 200"), LogLevel::Info);
    }

    #[test]
    fn test_filter_mode_cycle() {
        let mode = LogFilterMode::All;
        assert_eq!(mode.cycle(), LogFilterMode::WarnAndAbove);
        assert_eq!(mode.cycle().cycle(), LogFilterMode::ErrorOnly);
        assert_eq!(mode.cycle().cycle().cycle(), LogFilterMode::All);
    }

    #[test]
    fn test_grep_filter() {
        let filter = GrepFilter::with_pattern("error".into());
        assert!(filter.matches("Error occurred"));
        assert!(filter.matches("An ERROR happened"));
        assert!(!filter.matches("All good"));

        let case_filter = GrepFilter::with_pattern("Error".into()).with_case_sensitive(true);
        assert!(case_filter.matches("Error occurred"));
        assert!(!case_filter.matches("error occurred"));
    }
}
