//! How tasks look on the web: glyphs and Tailwind classes for the shared
//! `WorkItemStatus` / `Priority` enums. Moved out of bruhs-models, which
//! holds no presentation.

use bruhs_models::{Priority, WorkItemStatus};

pub trait WorkItemStatusStyle {
    fn glyph(&self) -> &'static str;
    fn color_class(&self) -> &'static str;
    fn dot_bg_class(&self) -> &'static str;
}

impl WorkItemStatusStyle for WorkItemStatus {
    fn glyph(&self) -> &'static str {
        match self {
            Self::Backlog => "⬚",
            Self::Todo => "⃝",
            Self::InProgress => "◐",
            Self::Done => "●",
            Self::Canceled => "⊘",
        }
    }

    fn color_class(&self) -> &'static str {
        match self {
            Self::Backlog => "text-rambutan-500",
            Self::Todo => "text-blueberry-400",
            Self::InProgress => "text-durian-400",
            Self::Done => "text-pandan-400",
            Self::Canceled => "text-rambutan-500",
        }
    }

    fn dot_bg_class(&self) -> &'static str {
        // Floor at rambutan-400/500 — the 600/700 steps sit below 3:1
        // against the longan canvas and render as invisible dots.
        match self {
            Self::Backlog => "bg-rambutan-400",
            Self::Todo => "bg-rambutan-300",
            Self::InProgress => "bg-persimmon-400",
            Self::Done => "bg-pandan-400",
            Self::Canceled => "bg-rambutan-500",
        }
    }
}

pub trait PriorityStyle {
    fn glyph(&self) -> &'static str;
    fn color_class(&self) -> &'static str;
}

impl PriorityStyle for Priority {
    fn glyph(&self) -> &'static str {
        match self {
            Self::Urgent => "‼",
            Self::High => "▲",
            Self::Medium => "◆",
            Self::Low => "▼",
            Self::None => "—",
        }
    }

    fn color_class(&self) -> &'static str {
        match self {
            Self::Urgent => "text-lychee-400",
            Self::High => "text-persimmon-400",
            Self::Medium => "text-durian-400",
            Self::Low => "text-blueberry-400",
            Self::None => "text-rambutan-500",
        }
    }
}
