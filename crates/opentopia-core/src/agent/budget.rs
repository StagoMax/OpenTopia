use crate::provider::ModelUsage;
use crate::settings::RolloutBudgetSettings;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextBudget {
    pub max_tokens: usize,
    pub used_tokens: usize,
    pub warnings: Vec<String>,
    /// Output, reasoning, and provider-framing headroom reserved before every
    /// provider round. It participates in pressure admission but is not part of
    /// the logical input estimate reported as `used_tokens`.
    #[serde(default)]
    pub reserved_generation_tokens: usize,
}

impl ContextBudget {
    pub fn new(max_tokens: usize) -> Self {
        Self {
            max_tokens,
            used_tokens: 0,
            warnings: Vec::new(),
            reserved_generation_tokens: 0,
        }
    }

    pub fn set_round_pressure(&mut self, reserved_generation_tokens: usize) {
        self.reserved_generation_tokens = reserved_generation_tokens;
    }

    pub fn pressure_tokens(&self) -> usize {
        self.used_tokens
            .saturating_add(self.reserved_generation_tokens)
    }

    pub fn pressure_percent(&self) -> usize {
        self.pressure_tokens().saturating_mul(100) / self.max_tokens.max(1)
    }

    pub fn requires_compaction(&self, threshold_percent: usize) -> bool {
        self.pressure_tokens().saturating_mul(100)
            >= self.max_tokens.saturating_mul(threshold_percent)
    }

    pub fn record_tokens(&mut self, tokens: usize) {
        self.used_tokens += tokens;
        let pressure_tokens = self.pressure_tokens();
        let usage_pct = pressure_tokens as f64 / self.max_tokens as f64;
        if usage_pct >= 0.90 && usage_pct < 0.95 {
            let msg = format!(
                "Context budget at {:.1}% (used {} / max {} tokens)",
                usage_pct * 100.0,
                pressure_tokens,
                self.max_tokens
            );
            if !self.warnings.iter().any(|w| w.contains("90%")) {
                self.warnings.push(msg);
            }
        } else if usage_pct >= 0.95 && usage_pct < 1.0 {
            let msg = format!(
                "Context budget critically high at {:.1}% (used {} / max {} tokens)",
                usage_pct * 100.0,
                pressure_tokens,
                self.max_tokens
            );
            if !self.warnings.iter().any(|w| w.contains("95%")) {
                self.warnings.push(msg);
            }
        }
    }

    pub fn is_exceeded(&self) -> bool {
        self.used_tokens >= self.max_tokens
    }

    pub fn estimate_tokens(text: &str) -> usize {
        crate::model_context::estimate_tokens(text)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RolloutBudget {
    settings: RolloutBudgetSettings,
    weighted_tokens_used: f64,
    delivered_reminders: u8,
}

impl RolloutBudget {
    pub(super) fn new(settings: RolloutBudgetSettings) -> Self {
        Self {
            settings,
            weighted_tokens_used: 0.0,
            delivered_reminders: 0,
        }
    }

    pub(super) fn record_usage(&mut self, usage: &ModelUsage) {
        let cached_input = usage.cached_input_tokens.unwrap_or_default();
        let uncached_input = usage.input_tokens.saturating_sub(cached_input);
        self.weighted_tokens_used += usage.output_tokens as f64
            * self.settings.sampling_token_weight
            + uncached_input as f64 * self.settings.prefill_token_weight;
    }

    pub(super) fn is_exhausted(&self) -> bool {
        self.weighted_tokens_used >= self.settings.limit_tokens as f64
    }

    pub(super) fn remaining_tokens(&self) -> u64 {
        (self.settings.limit_tokens as f64 - self.weighted_tokens_used)
            .max(0.0)
            .floor() as u64
    }

    /// Returns the reminder that is due without consuming it.
    ///
    /// Delivery is confirmed separately through [`RolloutBudget::mark_reminder_delivered`]
    /// so a round that is cancelled or fails before the reminder reaches the model
    /// redelivers it instead of dropping it silently.
    pub(super) fn pending_reminder(&self) -> Option<RolloutBudgetReminder> {
        let remaining = self.remaining_tokens();
        let level = if remaining <= self.settings.limit_tokens / 10 {
            2
        } else if remaining <= self.settings.limit_tokens / 4 {
            1
        } else {
            0
        };
        if level == 0 || level <= self.delivered_reminders {
            return None;
        }
        Some(RolloutBudgetReminder {
            level,
            content: format!(
                "[Rollout budget]\nApproximately {remaining} weighted tokens remain in this turn. Keep the original goal in view, prioritize the highest-value remaining work, and avoid unnecessary tool calls."
            ),
        })
    }

    pub(super) fn mark_reminder_delivered(&mut self, reminder: &RolloutBudgetReminder) {
        self.delivered_reminders = self.delivered_reminders.max(reminder.level);
    }
}

#[derive(Debug, Clone)]
pub(super) struct RolloutBudgetReminder {
    pub(super) level: u8,
    pub(super) content: String,
}
