//! Compiled include/exclude sets with the original byte-oriented glob semantics.
use anyhow::{Context, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use regex::bytes::RegexSet;

struct PatternSet {
    globs: GlobSet,
    recursive: RegexSet,
}

impl PatternSet {
    fn new(patterns: &[&str]) -> Result<Self> {
        let mut globs = GlobSetBuilder::new();
        let mut recursive = Vec::new();
        for pattern in patterns {
            let normalized = pattern.replace('\\', "/");
            let normalized = normalized.trim_start_matches("./");
            if normalized.contains("**") {
                // The old **/ consumes any prefix, including one that does not
                // end in '/'. globset's directory-component semantics differ.
                recursive.push(recursive_regex(normalized));
            } else {
                let escaped = normalized.chars().fold(String::new(), |mut text, ch| {
                    if matches!(ch, '[' | ']' | '{' | '}') {
                        text.push('\\');
                    }
                    text.push(ch);
                    text
                });
                globs.add(
                    GlobBuilder::new(&escaped)
                        .literal_separator(true)
                        .backslash_escape(true)
                        .build()
                        .context("failed to compile source glob")?,
                );
            }
        }
        Ok(Self {
            globs: globs.build().context("failed to compile source glob set")?,
            recursive: RegexSet::new(recursive)
                .context("failed to compile recursive source globs")?,
        })
    }

    fn is_match(&self, relative: &str) -> bool {
        self.globs.is_match(relative) || self.recursive.is_match(relative.as_bytes())
    }
}

fn recursive_regex(pattern: &str) -> String {
    let mut chars = pattern.chars().peekable();
    let mut regex = String::from("\\A(?s-u:");
    while let Some(ch) = chars.next() {
        match ch {
            '*' if chars.peek() == Some(&'*') => {
                while chars.peek() == Some(&'*') {
                    chars.next();
                }
                if chars.peek() == Some(&'/') {
                    chars.next();
                }
                regex.push_str(".*");
            }
            '*' => regex.push_str("[^/]*"),
            '?' => regex.push_str("[^/]"),
            ch => regex.push_str(&regex::escape(&ch.to_string())),
        }
    }
    regex.push_str(")\\z");
    regex
}

struct Rules {
    full: PatternSet,
    basename: PatternSet,
}
impl Rules {
    fn new(patterns: &[String]) -> Result<Self> {
        Ok(Self {
            full: PatternSet::new(&patterns.iter().map(String::as_str).collect::<Vec<_>>())?,
            // Deliberately check the original pattern, before normalization:
            // './foo.rs' was always root-relative, while 'foo.rs' also matched
            // a basename in any directory.
            basename: PatternSet::new(
                &patterns
                    .iter()
                    .filter(|pattern| !pattern.contains('/'))
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            )?,
        })
    }
    fn is_match(&self, relative: &str) -> bool {
        self.full.is_match(relative)
            || relative
                .rsplit('/')
                .next()
                .is_some_and(|name| self.basename.is_match(name))
    }
}

pub struct PathFilters {
    include_all: bool,
    exclude_empty: bool,
    include: Rules,
    exclude: Rules,
}
impl PathFilters {
    pub fn new(include: &[String], exclude: &[String]) -> Result<Self> {
        Ok(Self {
            include_all: include.is_empty(),
            exclude_empty: exclude.is_empty(),
            include: Rules::new(include)?,
            exclude: Rules::new(exclude)?,
        })
    }
    pub fn allows(&self, relative: &str) -> bool {
        (self.include_all || self.include.is_match(relative))
            && (self.exclude_empty || !self.exclude.is_match(relative))
    }
}

#[cfg(test)]
#[path = "../tests/support/legacy_filters.rs"]
mod legacy;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_recursive_literals_unicode_and_basename_compatibility() {
        let mut patterns = vec![
            "",
            "*",
            "**",
            "***",
            "**/",
            "**/*.rs",
            "src/**/*.rs",
            "src/**/main.rs",
            "src/**",
            "a**b",
            "a***?b",
            "foo.rs",
            "./foo.rs",
            "././foo.rs",
            "src\\*.rs",
            "[ab].rs",
            "{a,b}.rs",
            "[",
            "a]",
            "{",
            "?",
            "??",
            "???",
            "?.rs",
            "???.rs",
            "中文/*.rs",
            "**/中文?.rs",
            "a\nb*",
            "a\n**b",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        // Deterministic generated patterns exercise adjacent stars and literal
        // metacharacters, not just the standard examples supported by globset.
        let alphabet = ['a', 'b', '/', '*', '?', '[', ']', '{', '}', '.', '中'];
        let mut seed = 13u64;
        for size in 1..=6 {
            for _ in 0..16 {
                let mut pattern = String::new();
                for _ in 0..size {
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                    pattern.push(alphabet[(seed >> 32) as usize % alphabet.len()]);
                }
                patterns.push(pattern);
            }
        }
        let paths = [
            "",
            "a",
            "b",
            "a/b",
            "aa/bb",
            "src/main.rs",
            "src/nested/main.rs",
            "src/prefixmain.rs",
            "foo.rs",
            "nested/foo.rs",
            "nested/FOO.rs",
            "[ab].rs",
            "{a,b}.rs",
            "a]",
            "[",
            "{",
            "中",
            "中文",
            "中.rs",
            "中文/main.rs",
            "src/中文a.rs",
            "a\nb\nc",
            "a\nb",
            "main.rs\n",
            "a/",
            "/a",
            "///",
        ];
        for pattern in &patterns {
            let includes = vec![pattern.clone()];
            let include = PathFilters::new(&includes, &[]).unwrap();
            let exclude = PathFilters::new(&[], &includes).unwrap();
            for path in paths {
                assert_eq!(
                    include.allows(path),
                    legacy::matches_filters(path, &includes, &[]),
                    "include {pattern:?}: {path:?}"
                );
                assert_eq!(
                    exclude.allows(path),
                    legacy::matches_filters(path, &[], &includes),
                    "exclude {pattern:?}: {path:?}"
                );
            }
        }
    }

    #[test]
    fn multiple_patterns_keep_exclusion_priority_and_root_relative_paths() {
        let include = ["src/**/*.rs", "README.md", "[literal].rs"].map(str::to_owned);
        let exclude = ["generated.rs", "src/private/**", "README.md"].map(str::to_owned);
        let filters = PathFilters::new(&include, &exclude).unwrap();
        for path in [
            "src/main.rs",
            "src/nested/generated.rs",
            "src/private/main.rs",
            "docs/README.md",
            "src/[literal].rs",
            "src/literal.rs",
        ] {
            assert_eq!(
                filters.allows(path),
                legacy::matches_filters(path, &include, &exclude),
                "{path}"
            );
        }
    }
}
