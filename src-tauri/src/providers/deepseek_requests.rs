//! DSH native request correlation. No prompt/answer text is retained.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::domain::WaitingReason;

fn identifier(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 240 && !s.chars().any(char::is_control))
}

#[derive(Default)]
pub(super) struct Requests {
    questions: BTreeMap<String, BTreeSet<String>>,
    approvals: BTreeSet<String>,
    settled: BTreeSet<String>,
    invalid_calls: BTreeSet<String>,
}

impl Requests {
    pub fn apply(&mut self, kind: &str, event: &Value) -> Option<()> {
        let data = event.get("data")?;
        match kind {
            "tool/call" if data.get("name")?.as_str()? == "ask_user_question" => {
                let call = identifier(data.get("callId")?)?;
                let ids = question_ids(data.get("arguments"));
                let Some(ids) = ids else {
                    // Tool calls are logged before host argument validation.
                    // A malformed call is not an actual displayed question.
                    if !self.settled.contains(&format!("question:{call}")) {
                        self.invalid_calls.insert(call.into());
                    }
                    return Some(());
                };
                if !self.settled.contains(&format!("question:{call}")) {
                    self.questions.entry(call.into()).or_insert(ids);
                }
            }
            "tool/result" => {
                let message = data.get("message")?;
                let call = message
                    .get("toolCallId")
                    .or_else(|| message.pointer("/source/callId"))?
                    .as_str()?;
                if message.get("isError").and_then(Value::as_bool) == Some(true) {
                    if self.invalid_calls.remove(call) {
                        self.settled.insert(format!("question:{call}"));
                    }
                    if self.questions.remove(call).is_some() {
                        self.settled.insert(format!("question:{call}"));
                    }
                } else if let Some(pending) = self.questions.get_mut(call) {
                    for part in message.get("content")?.as_array()? {
                        let Some(answer) = part
                            .get("text")
                            .and_then(Value::as_str)
                            .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        else {
                            continue;
                        };
                        // An accepted/queued ACK is not an answer. A partial
                        // result settles only the question IDs actually returned.
                        if let Some(answers) = answer.get("answers").and_then(Value::as_array) {
                            for answer in answers {
                                if answer.get("selected").is_some_and(Value::is_array)
                                    || answer.get("custom").is_some_and(Value::is_string)
                                {
                                    if let Some(id) = answer.get("id").and_then(Value::as_str) {
                                        pending.remove(id);
                                    }
                                }
                            }
                        }
                    }
                    if pending.is_empty() {
                        self.questions.remove(call);
                        self.settled.insert(format!("question:{call}"));
                    }
                }
            }
            "approval/asked" => {
                let id = identifier(data.get("id")?)?;
                if !self.settled.contains(&format!("approval:{id}")) {
                    self.approvals.insert(id.into());
                }
            }
            "approval/decided"
                if data
                    .get("outcome")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty()) =>
            {
                if let Some(id) = data.get("id").and_then(Value::as_str) {
                    if self.approvals.remove(id) {
                        self.settled.insert(format!("approval:{id}"));
                    }
                }
            }
            _ => {}
        }
        Some(())
    }

    pub fn bounded(&self) -> bool {
        self.approvals.len()
            + self.invalid_calls.len()
            + self.questions.values().map(BTreeSet::len).sum::<usize>()
            <= 256
            && self.settled.len() <= 4096
    }

    pub fn reliable(&self) -> bool {
        self.invalid_calls.is_empty()
    }

    pub fn pending(&self) -> (Option<WaitingReason>, Vec<String>) {
        let reason = if !self.approvals.is_empty() {
            Some(WaitingReason::Approval)
        } else if !self.questions.is_empty() {
            Some(WaitingReason::Question)
        } else {
            None
        };
        let ids =
            self.approvals
                .iter()
                .map(|id| format!("approval:{id}"))
                .chain(self.questions.iter().flat_map(|(call, ids)| {
                    ids.iter().map(move |id| format!("question:{call}:{id}"))
                }))
                .collect();
        (reason, ids)
    }
}

fn question_ids(arguments: Option<&Value>) -> Option<BTreeSet<String>> {
    let arguments = arguments?;
    let decoded;
    let arguments = if let Some(text) = arguments.as_str() {
        decoded = serde_json::from_str::<Value>(text).ok()?;
        &decoded
    } else {
        arguments
    };
    let ids: BTreeSet<_> = arguments
        .get("questions")?
        .as_array()?
        .iter()
        .map(|q| identifier(q.get("id")?))
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .map(str::to_owned)
        .collect();
    (!ids.is_empty()).then_some(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn invalid_host_call_is_not_a_question_and_only_its_failure_recovers_collection() {
        let mut r = Requests::default();
        let invalid = json!({"data":{"name":"ask_user_question","callId":"bad-call","arguments":{"questions":"invalid JSON array"}}});
        r.apply("tool/call", &invalid);
        assert!(!r.reliable());
        assert!(r.pending().1.is_empty());
        r.apply(
            "tool/result",
            &json!({"data":{"message":{"toolCallId":"other","isError":true}}}),
        );
        r.apply("tool/result",&json!({"data":{"message":{"toolCallId":"bad-call","content":[{"text":"{\"accepted\":true}"}]}}}));
        assert!(!r.reliable());
        r.apply(
            "tool/result",
            &json!({"data":{"message":{"toolCallId":"bad-call","isError":true}}}),
        );
        assert!(r.reliable());
        assert!(r.pending().1.is_empty());
        r.apply("tool/call", &invalid);
        assert!(
            r.reliable(),
            "duplicate failed call does not resurrect uncertainty"
        );
    }

    #[test]
    fn questions_require_matching_answers_not_ack_tool_return_or_prose() {
        let mut r = Requests::default();
        r.apply("tool/call", &json!({"data":{"name":"ask_user_question","callId":"c1","arguments":"{\"questions\":[{\"id\":\"one\"},{\"id\":\"two\"}]}"}}));
        for event in [
            json!({"toolCallId":"other","content":[{"text":"{\"answers\":[{\"id\":\"one\",\"selected\":[]}]}"}]}),
            json!({"toolCallId":"c1","content":[{"text":"{\"accepted\":true}"}]}),
        ] {
            r.apply("tool/result", &json!({"data":{"message":event}}));
            assert_eq!(r.pending().1.len(), 2);
        }
        r.apply("user/message", &json!({"data":{"text":"answer"}}));
        for id in ["one", "two"] {
            r.apply("tool/result", &json!({"data":{"message":{"toolCallId":"c1","content":[{"text":json!({"answers":[{"id":id,"selected":[],"custom":"response"}]}).to_string()}]}}}));
            assert_eq!(r.pending().1.len(), usize::from(id == "one"));
        }
        assert_eq!(r.pending().0, None);
    }

    #[test]
    fn approvals_are_explicit_and_decisions_match_ids() {
        let mut r = Requests::default();
        r.apply(
            "approval/asked",
            &json!({"data":{"id":"a1","callId":"tool"}}),
        );
        r.apply(
            "tool/result",
            &json!({"data":{"message":{"toolCallId":"tool","isError":false}}}),
        );
        r.apply(
            "approval/decided",
            &json!({"data":{"id":"other","outcome":"approve"}}),
        );
        assert_eq!(
            r.pending(),
            (Some(WaitingReason::Approval), vec!["approval:a1".into()])
        );
        r.apply(
            "approval/decided",
            &json!({"data":{"id":"a1","outcome":"deny"}}),
        );
        assert!(r.pending().1.is_empty());
        r.apply(
            "approval/asked",
            &json!({"data":{"id":"a1","callId":"tool"}}),
        );
        assert!(
            r.pending().1.is_empty(),
            "a duplicated settled request cannot reopen"
        );
    }
}
