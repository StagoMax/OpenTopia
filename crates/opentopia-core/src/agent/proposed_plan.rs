use crate::model::MessagePart;

const OPEN_TAG: &str = "<proposed_plan>";
const CLOSE_TAG: &str = "</proposed_plan>";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParserState {
    Normal,
    ProposedPlan,
}

/// Removes proposed-plan delimiters from streamed model text without waiting
/// for the complete response. A short suffix is retained when a delimiter is
/// split across provider chunks.
#[derive(Debug, Default)]
pub(super) struct ProposedPlanStreamParser {
    state: ParserState,
    pending: String,
}

impl Default for ParserState {
    fn default() -> Self {
        Self::Normal
    }
}

impl ProposedPlanStreamParser {
    pub(super) fn push_str(&mut self, chunk: &str) -> String {
        self.pending.push_str(chunk);
        let mut visible = String::new();

        loop {
            let delimiter = match self.state {
                ParserState::Normal => OPEN_TAG,
                ParserState::ProposedPlan => CLOSE_TAG,
            };
            if let Some(index) = self.pending.find(delimiter) {
                visible.push_str(&self.pending[..index]);
                self.pending.drain(..index + delimiter.len());
                self.state = match self.state {
                    ParserState::Normal => ParserState::ProposedPlan,
                    ParserState::ProposedPlan => ParserState::Normal,
                };
                continue;
            }

            let retained = longest_suffix_prefix(&self.pending, delimiter);
            let ready = self.pending.len() - retained;
            visible.push_str(&self.pending[..ready]);
            self.pending.drain(..ready);
            return visible;
        }
    }

    pub(super) fn finish(mut self) -> String {
        std::mem::take(&mut self.pending)
    }
}

/// Converts the official Plan-mode block into a first-class message part.
/// Ordinary text before or after the block remains ordered around it. An
/// unterminated block is treated as a plan through end-of-message, matching the
/// streaming parser's best-effort behavior.
pub(super) fn proposed_plan_message_parts(text: String) -> Vec<MessagePart> {
    let mut parts = Vec::new();
    let mut remaining = text.as_str();
    let mut found_plan = false;

    while let Some(open_index) = remaining.find(OPEN_TAG) {
        found_plan = true;
        push_text_part(&mut parts, &remaining[..open_index]);
        let plan_start = open_index + OPEN_TAG.len();
        let after_open = &remaining[plan_start..];
        if let Some(close_index) = after_open.find(CLOSE_TAG) {
            parts.push(MessagePart::ProposedPlan {
                text: after_open[..close_index].to_string(),
            });
            remaining = &after_open[close_index + CLOSE_TAG.len()..];
        } else {
            parts.push(MessagePart::ProposedPlan {
                text: after_open.to_string(),
            });
            remaining = "";
            break;
        }
    }

    if !found_plan {
        return vec![MessagePart::Text { text }];
    }
    push_text_part(&mut parts, remaining);
    parts
}

fn push_text_part(parts: &mut Vec<MessagePart>, text: &str) {
    if !text.is_empty() {
        parts.push(MessagePart::Text {
            text: text.to_string(),
        });
    }
}

fn longest_suffix_prefix(value: &str, delimiter: &str) -> usize {
    let maximum = value.len().min(delimiter.len().saturating_sub(1));
    (1..=maximum)
        .rev()
        .find(|length| value.ends_with(&delimiter[..*length]))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_parser_removes_tags_split_across_chunks() {
        let mut parser = ProposedPlanStreamParser::default();
        let visible = [
            parser.push_str("Intro\n<prop"),
            parser.push_str("osed_plan>\n- inspect\n</proposed_"),
            parser.push_str("plan>\nOutro"),
            parser.finish(),
        ]
        .concat();

        assert_eq!(visible, "Intro\n\n- inspect\n\nOutro");
    }

    #[test]
    fn final_parser_preserves_text_and_extracts_plan_part() {
        let parts = proposed_plan_message_parts(
            "Intro\n<proposed_plan>\n- inspect\n</proposed_plan>\nOutro".to_string(),
        );

        assert!(matches!(
            parts.as_slice(),
            [
                MessagePart::Text { text: intro },
                MessagePart::ProposedPlan { text: plan },
                MessagePart::Text { text: outro }
            ] if intro == "Intro\n" && plan == "\n- inspect\n" && outro == "\nOutro"
        ));
    }

    #[test]
    fn final_parser_leaves_ordinary_messages_unchanged() {
        let parts = proposed_plan_message_parts("ordinary response".to_string());
        assert!(matches!(
            parts.as_slice(),
            [MessagePart::Text { text }] if text == "ordinary response"
        ));
    }
}
