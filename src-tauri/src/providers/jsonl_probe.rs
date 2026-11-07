//! Bounded streaming JSON validation/projection for oversized non-lifecycle records.
//! Large strings are validated without retaining their contents.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Root,
    Payload,
    Item,
    Other,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Key {
    Type,
    Payload,
    Item,
    Other,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    KeyOrEnd,
    Key,
    Colon,
    ValueOrEnd,
    Value,
    CommaOrEnd,
}
struct Frame {
    object: bool,
    role: Role,
    phase: Phase,
    key: Key,
    seen: u8,
}
enum Token {
    String {
        key: bool,
        capture: Option<Vec<u8>>,
        escape: u8,
        hex: u16,
        digits: u8,
        high: bool,
        utf8: u32,
        need: u8,
        min: u32,
    },
    Atom(Vec<u8>),
}

pub(super) struct EnvelopeProbe {
    stack: Vec<Frame>,
    token: Option<Token>,
    valid: bool,
    started: bool,
    done: bool,
    root_object: bool,
    root: Option<String>,
    payload: Option<String>,
    item: Option<String>,
}
impl Default for EnvelopeProbe {
    fn default() -> Self {
        Self {
            stack: Vec::new(),
            token: None,
            valid: true,
            started: false,
            done: false,
            root_object: false,
            root: None,
            payload: None,
            item: None,
        }
    }
}
impl EnvelopeProbe {
    pub(super) fn feed(&mut self, bytes: &[u8]) {
        let mut at = 0;
        while at < bytes.len() && self.valid {
            // Most diffs are ordinary ASCII. Validate those runs without
            // moving the token state once for every character.
            if matches!(
                self.token,
                Some(Token::String {
                    capture: None,
                    escape: 0,
                    digits: 0,
                    need: 0,
                    high: false,
                    ..
                })
            ) {
                let run = bytes[at..]
                    .iter()
                    .position(|b| *b < 0x20 || *b >= 0x80 || matches!(*b, b'"' | b'\\'))
                    .unwrap_or(bytes.len() - at);
                at += run;
                if at == bytes.len() {
                    break;
                }
            }
            self.byte(bytes[at]);
            at += 1;
        }
    }
    fn value_done(&mut self) {
        if let Some(frame) = self.stack.last_mut() {
            frame.phase = Phase::CommaOrEnd;
            frame.key = Key::Other;
        } else {
            self.done = true;
        }
    }
    fn string_done(&mut self, key: bool, capture: Option<Vec<u8>>) {
        let text = match capture {
            Some(raw) => match serde_json::from_slice::<String>(&raw) {
                Ok(s) => Some(s),
                Err(_) => {
                    self.valid = false;
                    return;
                }
            },
            None => None,
        };
        let Some(frame) = self.stack.last_mut() else {
            self.value_done();
            return;
        };
        if key {
            frame.key = match text.as_deref() {
                Some("type") => Key::Type,
                Some("payload") => Key::Payload,
                Some("item") => Key::Item,
                _ => Key::Other,
            };
            let bit = match (frame.role, frame.key) {
                (Role::Root | Role::Payload | Role::Item, Key::Type) => 1,
                (Role::Root, Key::Payload) | (Role::Payload, Key::Item) => 2,
                _ => 0,
            };
            if bit != 0 && frame.seen & bit != 0 {
                self.valid = false;
                return;
            }
            frame.seen |= bit;
            frame.phase = Phase::Colon;
        } else {
            if frame.key == Key::Type {
                match frame.role {
                    Role::Root => self.root = text,
                    Role::Payload => self.payload = text,
                    Role::Item => self.item = text,
                    Role::Other => {}
                }
            }
            self.value_done();
        }
    }
    fn byte(&mut self, byte: u8) {
        if let Some(token) = self.token.take() {
            match token {
                Token::Atom(mut raw) => {
                    if byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.') {
                        if raw.len() >= 64 {
                            self.valid = false;
                            return;
                        }
                        raw.push(byte);
                        self.token = Some(Token::Atom(raw));
                        return;
                    }
                    if serde_json::from_slice::<serde::de::IgnoredAny>(&raw).is_err() {
                        self.valid = false;
                        return;
                    }
                    self.value_done();
                    self.byte(byte);
                    return;
                }
                Token::String {
                    key,
                    mut capture,
                    mut escape,
                    mut hex,
                    mut digits,
                    mut high,
                    mut utf8,
                    mut need,
                    mut min,
                } => {
                    if capture.as_ref().is_some_and(|raw| raw.len() >= 256) {
                        capture = None;
                    } else if let Some(raw) = &mut capture {
                        raw.push(byte);
                    }
                    let mut finished = false;
                    if need > 0 {
                        if byte & 0xc0 != 0x80 {
                            self.valid = false;
                            return;
                        }
                        utf8 = (utf8 << 6) | u32::from(byte & 0x3f);
                        need -= 1;
                        if need == 0
                            && (utf8 < min || utf8 > 0x10ffff || (0xd800..=0xdfff).contains(&utf8))
                        {
                            self.valid = false;
                            return;
                        }
                    } else if digits > 0 {
                        let digit = match byte {
                            b'0'..=b'9' => byte - b'0',
                            b'a'..=b'f' => byte - b'a' + 10,
                            b'A'..=b'F' => byte - b'A' + 10,
                            _ => {
                                self.valid = false;
                                return;
                            }
                        };
                        hex = (hex << 4) | u16::from(digit);
                        digits -= 1;
                        if digits == 0 {
                            if high {
                                if !(0xdc00..=0xdfff).contains(&hex) {
                                    self.valid = false;
                                    return;
                                }
                                high = false;
                            } else if (0xd800..=0xdbff).contains(&hex) {
                                high = true;
                                escape = 2;
                            } else if (0xdc00..=0xdfff).contains(&hex) {
                                self.valid = false;
                                return;
                            }
                        }
                    } else if escape != 0 {
                        match escape {
                            2 => {
                                if byte != b'\\' {
                                    self.valid = false;
                                    return;
                                }
                                escape = 3;
                            }
                            3 => {
                                if byte != b'u' {
                                    self.valid = false;
                                    return;
                                }
                                escape = 0;
                                digits = 4;
                                hex = 0;
                            }
                            _ => {
                                escape = 0;
                                match byte {
                                    b'u' => {
                                        digits = 4;
                                        hex = 0;
                                    }
                                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {}
                                    _ => {
                                        self.valid = false;
                                        return;
                                    }
                                }
                            }
                        }
                    } else {
                        match byte {
                            b'"' => finished = true,
                            b'\\' => escape = 1,
                            0..=31 => {
                                self.valid = false;
                                return;
                            }
                            0x80..=0xff => {
                                let (n, value, minimum) = match byte {
                                    0xc2..=0xdf => (1, u32::from(byte & 0x1f), 0x80),
                                    0xe0..=0xef => (2, u32::from(byte & 0x0f), 0x800),
                                    0xf0..=0xf4 => (3, u32::from(byte & 7), 0x10000),
                                    _ => {
                                        self.valid = false;
                                        return;
                                    }
                                };
                                need = n;
                                utf8 = value;
                                min = minimum;
                            }
                            _ => {}
                        }
                    }
                    if finished {
                        self.string_done(key, capture);
                    } else {
                        self.token = Some(Token::String {
                            key,
                            capture,
                            escape,
                            hex,
                            digits,
                            high,
                            utf8,
                            need,
                            min,
                        });
                    }
                    return;
                }
            }
        }
        if matches!(byte, b' ' | b'\r' | b'\n' | b'\t') {
            return;
        }
        if self.done {
            self.valid = false;
            return;
        }
        let phase = self.stack.last().map_or(Phase::Value, |f| f.phase);
        match phase {
            Phase::KeyOrEnd | Phase::Key => {
                if byte == b'}' && phase == Phase::KeyOrEnd {
                    self.stack.pop();
                    self.value_done();
                } else if byte == b'"' {
                    let capture =
                        (self.stack.last().unwrap().role != Role::Other).then(|| vec![b'"']);
                    self.token = Some(Token::String {
                        key: true,
                        capture,
                        escape: 0,
                        hex: 0,
                        digits: 0,
                        high: false,
                        utf8: 0,
                        need: 0,
                        min: 0,
                    });
                } else {
                    self.valid = false;
                }
            }
            Phase::Colon => {
                if byte == b':' {
                    self.stack.last_mut().unwrap().phase = Phase::Value;
                } else {
                    self.valid = false;
                }
            }
            Phase::CommaOrEnd => {
                let frame = self.stack.last_mut().unwrap();
                if byte == b',' {
                    frame.phase = if frame.object {
                        Phase::Key
                    } else {
                        Phase::Value
                    };
                } else if byte == if frame.object { b'}' } else { b']' } {
                    self.stack.pop();
                    self.value_done();
                } else {
                    self.valid = false;
                }
            }
            Phase::ValueOrEnd if byte == b']' => {
                self.stack.pop();
                self.value_done();
            }
            Phase::Value | Phase::ValueOrEnd => {
                let role = if !self.started {
                    self.started = true;
                    self.root_object = byte == b'{';
                    Role::Root
                } else {
                    self.stack
                        .last()
                        .map_or(Role::Other, |f| match (f.role, f.key) {
                            (Role::Root, Key::Payload) => Role::Payload,
                            (Role::Payload, Key::Item) => Role::Item,
                            _ => Role::Other,
                        })
                };
                match byte {
                    b'{' | b'[' => {
                        if self.stack.len() >= 128 {
                            self.valid = false;
                            return;
                        }
                        let object = byte == b'{';
                        self.stack.push(Frame {
                            object,
                            role: if object { role } else { Role::Other },
                            phase: if object {
                                Phase::KeyOrEnd
                            } else {
                                Phase::ValueOrEnd
                            },
                            key: Key::Other,
                            seen: 0,
                        });
                    }
                    b'"' => {
                        let capture = self
                            .stack
                            .last()
                            .is_some_and(|f| f.role != Role::Other && f.key == Key::Type)
                            .then(|| vec![b'"']);
                        self.token = Some(Token::String {
                            key: false,
                            capture,
                            escape: 0,
                            hex: 0,
                            digits: 0,
                            high: false,
                            utf8: 0,
                            need: 0,
                            min: 0,
                        });
                    }
                    b'-' | b'0'..=b'9' | b't' | b'f' | b'n' => {
                        self.token = Some(Token::Atom(vec![byte]))
                    }
                    _ => self.valid = false,
                }
            }
        }
    }
    pub(super) fn ignored(&self, allowed: &[&str]) -> bool {
        if !self.valid
            || !self.root_object
            || !self.done
            || !self.stack.is_empty()
            || self.token.is_some()
        {
            return false;
        }
        self.root
            .as_deref()
            .is_some_and(|root| allowed.contains(&root))
            || matches!(
                (
                    self.root.as_deref(),
                    self.payload.as_deref(),
                    self.item.as_deref()
                ),
                (
                    Some("event_msg"),
                    Some("item_completed"),
                    Some("FileChange")
                )
            ) && allowed.contains(&"event_msg/item_completed/FileChange")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_projected_record_is_validated_across_every_chunk_boundary() {
        let value=br#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"FileChange","diff":"huge\\patch\uD83D\uDE00"}}}"#;
        for size in 1..value.len() {
            let mut p = EnvelopeProbe::default();
            for part in value.chunks(size) {
                p.feed(part);
            }
            assert!(p.ignored(&["event_msg/item_completed/FileChange"]));
        }
    }
    #[test]
    fn malformed_duplicate_or_lifecycle_events_cannot_be_ignored() {
        for value in [
            r#"{"type":"event_msg","payload":{"type":"task_complete","item":{"type":"FileChange"}}}"#,
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"FileChange"}},}"#,
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"FileChange","type":"Approval"}}}"#,
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"FileChange","text":"\q"}}}"#,
            r#"{"type":"event_msg","payload":{"type":"item_completed","item":{"type":"FileChange"}},"payload":{"type":"task_complete"}}"#,
        ] {
            let mut p = EnvelopeProbe::default();
            p.feed(value.as_bytes());
            assert!(!p.ignored(&["event_msg/item_completed/FileChange"]));
        }
    }
}
