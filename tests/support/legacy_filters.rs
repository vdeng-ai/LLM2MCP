// v0.4.1 behavior oracle for compatibility tests and opt-in benchmarks.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern = pattern.replace('\\', "/");
    let pattern = pattern.trim_start_matches("./").as_bytes();
    let text = text.as_bytes();
    let mut memo = std::collections::HashMap::<(usize, usize), bool>::new();

    fn matches(
        pattern: &[u8],
        text: &[u8],
        pi: usize,
        ti: usize,
        memo: &mut std::collections::HashMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(value) = memo.get(&(pi, ti)) {
            return *value;
        }
        let result = if pi == pattern.len() {
            ti == text.len()
        } else if pattern[pi] == b'*' && pi + 1 < pattern.len() && pattern[pi + 1] == b'*' {
            let mut next = pi + 2;
            while next < pattern.len() && pattern[next] == b'*' {
                next += 1;
            }
            if next < pattern.len() && pattern[next] == b'/' {
                matches(pattern, text, next + 1, ti, memo)
                    || (ti < text.len() && matches(pattern, text, pi, ti + 1, memo))
            } else {
                matches(pattern, text, next, ti, memo)
                    || (ti < text.len() && matches(pattern, text, pi, ti + 1, memo))
            }
        } else if pattern[pi] == b'*' {
            matches(pattern, text, pi + 1, ti, memo)
                || (ti < text.len() && text[ti] != b'/' && matches(pattern, text, pi, ti + 1, memo))
        } else if pattern[pi] == b'?' {
            ti < text.len() && text[ti] != b'/' && matches(pattern, text, pi + 1, ti + 1, memo)
        } else {
            ti < text.len()
                && pattern[pi] == text[ti]
                && matches(pattern, text, pi + 1, ti + 1, memo)
        };
        memo.insert((pi, ti), result);
        result
    }

    matches(pattern, text, 0, 0, &mut memo)
}

pub fn matches_pattern(pattern: &str, relative: &str) -> bool {
    glob_match(pattern, relative)
        || (!pattern.contains('/')
            && relative
                .rsplit('/')
                .next()
                .is_some_and(|name| glob_match(pattern, name)))
}

pub fn matches_filters(relative: &str, include: &[String], exclude: &[String]) -> bool {
    let included = include.is_empty()
        || include
            .iter()
            .any(|pattern| matches_pattern(pattern, relative));
    let excluded = exclude
        .iter()
        .any(|pattern| matches_pattern(pattern, relative));
    included && !excluded
}
