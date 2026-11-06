//! The fixed service routes. Account data cannot add or rewrite routes.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line {
    pub id: &'static str,
    pub base_url: &'static str,
    pub label_key: &'static str,
    pub transport: &'static str,
    pub priority: u16,
    pub first_byte_s: u64,
    pub idle_s: u64,
    pub stream_total_s: u64,
    pub non_stream_s: u64,
}

pub static LINES: [Line; 3] = [
    Line {
        id: "main",
        base_url: "https://aicliapi.com",
        label_key: "aicli.lines.main",
        transport: "cloudflare",
        priority: 100,
        first_byte_s: 100,
        idle_s: 110,
        stream_total_s: 600,
        non_stream_s: 110,
    },
    Line {
        id: "global",
        base_url: "https://aicli.qzz.io",
        label_key: "aicli.lines.global",
        transport: "cloudflare",
        priority: 90,
        first_byte_s: 100,
        idle_s: 110,
        stream_total_s: 600,
        non_stream_s: 110,
    },
    Line {
        id: "apac",
        base_url: "https://api.aicli.qzz.io",
        label_key: "aicli.lines.apac",
        transport: "cdn",
        priority: 80,
        first_byte_s: 300,
        idle_s: 300,
        stream_total_s: 600,
        non_stream_s: 600,
    },
];

pub fn line(id: &str) -> Option<&'static Line> {
    LINES.iter().find(|candidate| candidate.id == id)
}

/// C4-04：种子 provider id（`aicli-<line>`）反推线路 id；仅认注册表里的固定值。
pub fn line_id_of_provider(provider_id: &str) -> Option<&'static str> {
    let id = provider_id.strip_prefix("aicli-")?;
    LINES.iter().find(|line| line.id == id).map(|line| line.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_matches_all_owner_approved_fields() {
        let expected = [
            (
                "main",
                "https://aicliapi.com",
                "aicli.lines.main",
                "cloudflare",
                100,
                100,
                110,
                600,
                110,
            ),
            (
                "global",
                "https://aicli.qzz.io",
                "aicli.lines.global",
                "cloudflare",
                90,
                100,
                110,
                600,
                110,
            ),
            (
                "apac",
                "https://api.aicli.qzz.io",
                "aicli.lines.apac",
                "cdn",
                80,
                300,
                300,
                600,
                600,
            ),
        ];
        assert_eq!(LINES.len(), expected.len());
        for (
            actual,
            (
                id,
                base_url,
                label_key,
                transport,
                priority,
                first_byte_s,
                idle_s,
                stream_total_s,
                non_stream_s,
            ),
        ) in LINES.iter().zip(expected)
        {
            assert_eq!(actual.id, id);
            assert_eq!(actual.base_url, base_url);
            assert_eq!(actual.label_key, label_key);
            assert_eq!(actual.transport, transport);
            assert_eq!(actual.priority, priority);
            assert_eq!(actual.first_byte_s, first_byte_s);
            assert_eq!(actual.idle_s, idle_s);
            assert_eq!(actual.stream_total_s, stream_total_s);
            assert_eq!(actual.non_stream_s, non_stream_s);
            assert!(!actual.base_url.ends_with('/'));
            assert!(std::ptr::eq(line(id).unwrap(), actual));
        }
    }

    #[test]
    fn lookup_rejects_unknown_ids_without_normalizing_them() {
        for id in [
            "",
            "MAIN",
            " main",
            "main/",
            "https://aicliapi.com",
            "unknown",
        ] {
            assert!(line(id).is_none(), "unexpected route: {id}");
        }
    }
}
