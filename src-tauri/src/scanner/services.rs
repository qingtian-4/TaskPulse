//! Exclude Agent transport services, not words inside a task's prompt.
pub(super) fn codex_service(command: &str) -> bool {
    let args: Vec<String> = if command.contains('\0') {
        command
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    } else {
        let mut args = Vec::new();
        let mut word = String::new();
        let mut quote = None;
        for ch in command.chars() {
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() && matches!(ch, '\'' | '"') {
                quote = Some(ch);
            } else if quote.is_none() && ch.is_whitespace() {
                if !word.is_empty() {
                    args.push(std::mem::take(&mut word));
                }
            } else {
                word.push(ch);
            }
        }
        if !word.is_empty() {
            args.push(word);
        }
        args
    };
    let mut args = args.iter().skip(1);
    if args
        .clone()
        .next()
        .is_some_and(|arg| arg.ends_with("codex.js"))
    {
        args.next();
    }
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-c" | "--config" | "--enable" | "--disable" | "-C" | "--cd" | "-p" | "--profile"
            | "-m" | "--model" | "-a" | "--ask-for-approval" | "-s" | "--sandbox" | "--add-dir"
            | "-i" | "--image" => {
                args.next();
            }
            value if value.starts_with('-') => continue,
            value => return matches!(value, "app-server" | "mcp-server"),
        }
    }
    false
}

pub(super) fn owned_descendant(
    mut pid: u32,
    parents: &std::collections::HashMap<u32, u32>,
    owner: u32,
) -> bool {
    for _ in 0..64 {
        if pid == owner {
            return true;
        }
        match parents.get(&pid) {
            Some(&parent) if parent != pid => pid = parent,
            _ => break,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn services_are_not_user_sessions_or_prompt_substrings() {
        for command in [
            r#""C:\Program Files\Codex\codex.exe" app-server --stdio"#,
            "codex\0-c\0x=true\0app-server\0",
            "node /bin/codex.js mcp-server",
        ] {
            assert!(codex_service(command), "{command}");
        }
        for command in [
            "codex",
            "codex exec 'fix app-server'",
            "codex 'app-server is broken'",
            "codex -c model=app-server exec task",
        ] {
            assert!(!codex_service(command), "{command}");
        }
    }
    #[test]
    fn only_our_own_helpers_are_excluded_by_parentage() {
        let parents = [(2, 1), (3, 2), (4, 1), (9, 9)].into();
        assert!(owned_descendant(3, &parents, 2));
        assert!(!owned_descendant(4, &parents, 2));
        assert!(!owned_descendant(9, &parents, 2));
    }
}
