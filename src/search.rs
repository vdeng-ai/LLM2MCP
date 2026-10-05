pub fn keywords(task: &str) -> Vec<String> {
    let mut words = task
        .split(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
        .filter(|word| word.chars().count() >= 2)
        .map(str::to_lowercase)
        .collect::<Vec<_>>();
    for (chinese, aliases) in [
        ("登录", "login auth session"),
        ("认证", "auth authentication"),
        ("权限", "permission authorization"),
        ("缓存", "cache"),
        ("任务", "job task worker"),
        ("队列", "queue scheduler"),
        ("并发", "concurrency mutex lock"),
        ("取消", "cancel cancellation"),
        ("模型", "model llm"),
        ("请求", "request http client"),
        ("配置", "config settings"),
        ("数据库", "database sql"),
        ("文档", "docs document"),
        ("更新", "update updater"),
        ("支付", "payment checkout"),
        ("上传", "upload"),
        ("下载", "download"),
        ("超时", "timeout"),
        ("重试", "retry"),
        ("文件", "file filesystem"),
        ("解析", "parse parser"),
        ("日志", "log logging"),
        ("界面", "gui ui"),
        ("网络", "network http"),
    ] {
        if task.contains(chinese) {
            words.extend(aliases.split_whitespace().map(str::to_owned));
        }
    }
    words.retain(|word| {
        ![
            "the",
            "and",
            "for",
            "with",
            "from",
            "this",
            "that",
            "what",
            "why",
            "how",
            "find",
            "fix",
            "plan",
            "analyze",
            "review",
            "implement",
            "code",
            "project",
            "issue",
            "problem",
            "need",
            "should",
        ]
        .contains(&word.as_str())
    });
    words.sort();
    words.dedup();
    words
}
pub fn score(words: &[String], text: &str) -> usize {
    let text = text.to_lowercase();
    words
        .iter()
        .filter(|word| text.contains(word.as_str()))
        .count()
}
#[cfg(test)]
mod tests {
    #[test]
    fn chinese_intent_matches_identifiers() {
        assert!(
            super::score(
                &super::keywords("分析登录缓存失效的根因"),
                "src/auth/session_cache.rs"
            ) >= 3
        );
    }
}
