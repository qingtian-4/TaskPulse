//! Native question request correlation. Dispatch ACK is not a human answer.
//! Retain only IDs and outstanding indexes, never question or answer text.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq)]
enum Request {
    Resolved,
    Sync,
    Async {
        accepted: bool,
        remaining: BTreeSet<usize>,
    },
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct Questions {
    pending: BTreeMap<String, Request>,
}

impl Questions {
    pub fn called(&mut self, payload: &Value) {
        let Some(name) = payload
            .get("name")
            .and_then(Value::as_str)
            .and_then(|s| s.rsplit('.').next())
        else {
            return;
        };
        let Some(id) = payload
            .get("call_id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            return;
        };
        let request = match name {
            "request_user_input" => Request::Sync,
            "request_user_input_async" => {
                let Some(args) = payload.get("arguments").or_else(|| payload.get("input")) else {
                    return;
                };
                let parsed;
                let args = if let Some(raw) = args.as_str() {
                    let Ok(value) = serde_json::from_str::<Value>(raw) else {
                        return;
                    };
                    parsed = value;
                    &parsed
                } else {
                    args
                };
                let Some(items) = args
                    .get("questions")
                    .and_then(Value::as_array)
                    .filter(|q| !q.is_empty())
                else {
                    return;
                };
                Request::Async {
                    accepted: false,
                    remaining: (0..items.len()).collect(),
                }
            }
            _ => return,
        };
        // A repeated native call cannot reopen already answered items.
        self.pending.entry(id.to_owned()).or_insert(request);
    }

    pub fn returned(&mut self, payload: &Value) {
        let Some(id) = payload.get("call_id").and_then(Value::as_str) else {
            return;
        };
        match self.pending.get_mut(id) {
            Some(Request::Sync) => {
                self.pending.insert(id.to_owned(), Request::Resolved);
            }
            Some(Request::Async { accepted, .. }) => {
                match payload.get("output").and_then(dispatch_ack) {
                    Some(true) => *accepted = true,
                    Some(false) => {
                        self.pending.insert(id.to_owned(), Request::Resolved);
                    }
                    None => {} // Unknown tool text is not evidence of an answer.
                }
            }
            Some(Request::Resolved) | None => {}
        }
    }

    /// Some(false) is a native reply envelope for an unrelated/already answered
    /// request. It must not be mistaken for an unrelated new user turn.
    pub fn replied(&mut self, payload: &Value) -> Option<bool> {
        if !matches!(
            payload.get("type").and_then(Value::as_str),
            Some("user_message")
        ) && !(payload.get("type").and_then(Value::as_str) == Some("message")
            && payload.get("role").and_then(Value::as_str) == Some("user"))
        {
            return None;
        }
        const BEGIN: &str = "<send_user_message_question_reply>";
        const END: &str = "</send_user_message_question_reply>";
        let texts = payload
            .get("message")
            .and_then(Value::as_str)
            .into_iter()
            .chain(
                payload
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.get("text").and_then(Value::as_str)),
            );
        let mut recognized = false;
        let mut changed = false;
        for text in texts {
            let Some(body) = text
                .trim()
                .strip_prefix(BEGIN)
                .and_then(|s| s.strip_suffix(END))
            else {
                continue;
            };
            let Ok(rows) = serde_json::from_str::<Vec<Value>>(body) else {
                continue;
            };
            recognized = true;
            for row in rows {
                if row.get("answer").and_then(Value::as_str).is_none() {
                    continue;
                }
                let Some(raw) = row.get("questionItemId").and_then(Value::as_str) else {
                    continue;
                };
                let Ok((method, call, index)) =
                    serde_json::from_str::<(String, String, usize)>(raw)
                else {
                    continue;
                };
                if method.rsplit('.').next() != Some("request_user_input_async") {
                    continue;
                }
                let remove =
                    if let Some(Request::Async { remaining, .. }) = self.pending.get_mut(&call) {
                        changed |= remaining.remove(&index);
                        remaining.is_empty()
                    } else {
                        false
                    };
                if remove {
                    self.pending.insert(call, Request::Resolved);
                }
            }
        }
        recognized.then_some(changed)
    }

    pub fn active_ids(&self) -> Vec<String> {
        self.pending
            .iter()
            .filter(|(_, request)| {
                matches!(
                    request,
                    Request::Sync | Request::Async { accepted: true, .. }
                )
            })
            .map(|(id, _)| id.clone())
            .collect()
    }

    pub fn has_active(&self) -> bool {
        self.pending.values().any(|request| {
            matches!(
                request,
                Request::Sync | Request::Async { accepted: true, .. }
            )
        })
    }

    pub fn finish_turn(&mut self) {
        // Accepted async questions remain on the host UI after ordinary turn
        // completion; only their actual replies resolve them.
        for request in self.pending.values_mut() {
            if matches!(request, Request::Sync) {
                *request = Request::Resolved;
            }
        }
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }
}

fn dispatch_ack(value: &Value) -> Option<bool> {
    if let Some(raw) = value.as_str() {
        return serde_json::from_str::<Value>(raw)
            .ok()?
            .get("accepted")?
            .as_bool();
    }
    if let Some(items) = value.as_array() {
        return items
            .iter()
            .filter(|item| {
                matches!(
                    item.get("type").and_then(Value::as_str),
                    Some("input_text" | "text")
                )
            })
            .filter_map(|item| item.get("text"))
            .find_map(dispatch_ack);
    }
    value.get("accepted").and_then(Value::as_bool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sync_questions_resolve_only_the_returned_call_and_async_dispatch_needs_acceptance() {
        let mut book = Questions::default();
        for id in ["a", "b"] {
            book.called(&json!({"name":"functions.request_user_input","call_id":id}));
        }
        book.returned(&json!({"call_id":"other"}));
        assert_eq!(book.active_ids(), vec!["a", "b"]);
        book.returned(&json!({"call_id":"a"}));
        assert_eq!(book.active_ids(), vec!["b"]);
        book.finish_turn();
        assert!(!book.has_active());
        book.called(&json!({"name":"request_user_input_async","call_id":"c","arguments":"{\"questions\":[{}]}"}));
        assert!(!book.has_active());
        // Native ACKs may be written after an ordinary completion. Ending the
        // turn cannot manufacture either acceptance or rejection of dispatch.
        book.finish_turn();
        book.returned(&json!({"call_id":"c","output":"some arbitrary text about accepted: true"}));
        assert!(!book.has_active());
        book.returned(&json!({"call_id":"c","output":"{\"accepted\":true}"}));
        assert_eq!(book.active_ids(), vec!["c"]);
    }

    #[test]
    fn resolved_question_ids_cannot_be_reopened_by_duplicate_dispatch_records() {
        let mut book = Questions::default();
        let call =
            json!({"name":"request_user_input_async","call_id":"a","arguments":{"questions":[{}]}});
        let ack = json!({"call_id":"a","output":{"accepted":true}});
        book.called(&call);
        book.returned(&ack);
        let reply =
            json!([{"questionItemId":"[\"request_user_input_async\",\"a\",0]","answer":"Yes"}]);
        let payload = json!({"type":"message","role":"user","content":[{"text":format!("<send_user_message_question_reply>{reply}</send_user_message_question_reply>")} ]});
        assert_eq!(book.replied(&payload), Some(true));
        book.called(&call);
        book.returned(&ack);
        assert!(!book.has_active());
        assert_eq!(book.replied(&payload), Some(false));
        book.clear();
        book.called(&call);
        book.returned(&ack);
        assert!(book.has_active());
    }

    #[test]
    fn command_text_similar_names_and_quoted_reply_tags_are_not_question_events() {
        let mut book = Questions::default();
        for name in [
            "request_user_input_async_debug",
            "exec_command",
            "request_user_input_async ",
        ] {
            book.called(&json!({"name":name,"call_id":"q","arguments":"{\"questions\":[{}]}"}));
        }
        assert!(!book.has_active());
        book.called(&json!({"name":"request_user_input_async","call_id":"q","arguments":{"questions":[{}]}}));
        book.returned(&json!({"call_id":"q","output":{"accepted":true}}));
        let text="<send_user_message_question_reply>[{\"questionItemId\":\"[\\\"request_user_input_async\\\",\\\"q\\\",0]\",\"answer\":\"Yes\"}]</send_user_message_question_reply>";
        for (role, text) in [
            ("assistant", text.to_string()),
            ("user", format!("Example: {text}")),
            ("user", text.replace(",0]", ",99]")),
        ] {
            assert_ne!(
                book.replied(&json!({"type":"message","role":role,"content":[{"text":text}]})),
                Some(true)
            );
            assert!(book.has_active());
        }
    }
}
