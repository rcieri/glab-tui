use chrono::{DateTime, Utc};

pub fn truncate(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        None => String::from(s),
        Some((idx, _)) => {
            let mut truncated = String::from(&s[..idx]);
            truncated.push_str("...");
            truncated
        }
    }
}

pub fn time_ago(date_str: &str) -> String {
    if let Ok(parsed_time) = date_str.parse::<DateTime<Utc>>() {
        let now = Utc::now();
        let duration = now.signed_duration_since(parsed_time);

        let days = duration.num_days();
        if days > 0 {
            if days == 1 {
                return "1 day ago".to_string();
            }
            return format!("{} days ago", days);
        }

        let hours = duration.num_hours();
        if hours > 0 {
            if hours == 1 {
                return "1 hr ago".to_string();
            }
            return format!("{} hrs ago", hours);
        }

        let minutes = duration.num_minutes();
        if minutes > 0 {
            if minutes == 1 {
                return "1 min ago".to_string();
            }
            return format!("{} mins ago", minutes);
        }

        "just now".to_string()
    } else {
        date_str.to_string()
    }
}

pub fn format_ref(r#ref: &str) -> String {
    if let Some(mr_id) = r#ref.strip_prefix("refs/merge-requests/").and_then(|s| s.strip_suffix("/merge")) {
        format!("MR !{}", mr_id)
    } else if let Some(mr_id) = r#ref.strip_prefix("refs/merge-requests/").and_then(|s| s.split('/').next()) {
        format!("MR !{}", mr_id)
    } else if let Some(branch) = r#ref.strip_prefix("refs/heads/") {
        branch.to_string()
    } else if let Some(tag) = r#ref.strip_prefix("refs/tags/") {
        tag.to_string()
    } else {
        r#ref.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_ref() {
        assert_eq!(format_ref("refs/merge-requests/123/merge"), "MR !123");
        assert_eq!(format_ref("refs/merge-requests/456/head"), "MR !456");
        assert_eq!(format_ref("refs/heads/feature/login"), "feature/login");
        assert_eq!(format_ref("refs/tags/v1.2.3"), "v1.2.3");
        assert_eq!(format_ref("main"), "main");
    }
}
