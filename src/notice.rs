use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// How long a notice stays on screen.
pub const NOTICE_LIFETIME: Duration = Duration::from_secs(5);

/// Bounds the queue during a burst of failures: only the newest notice of
/// each kind is ever drawn, and the oldest expire first anyway.
const MAX_QUEUED_NOTICES: usize = 32;

/// Second line of an error whose full CLI stderr is in the Terminal tab.
pub const CLI_LOG_HINT: &str = "Full details in the terminal log below";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeLevel {
    Info,
    Error,
}

/// Names one request a modal started. A notice carrying it is drawn inside
/// that modal while the modal still holds the same tag, and falls back to
/// the toast or the header bar once the modal is gone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RequestTag(u64);

impl RequestTag {
    pub fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Notice {
    level: NoticeLevel,
    text: String,
    detail: Option<String>,
    origin: Option<RequestTag>,
    raised_at: Instant,
}

impl Notice {
    pub fn info(text: impl Into<String>) -> Self {
        Self::new(NoticeLevel::Info, text.into())
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self::new(NoticeLevel::Error, text.into())
    }

    fn new(level: NoticeLevel, text: String) -> Self {
        Notice {
            level,
            text,
            detail: None,
            origin: None,
            raised_at: Instant::now(),
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn from_request(mut self, origin: RequestTag) -> Self {
        self.origin = Some(origin);
        self
    }

    pub fn level(&self) -> NoticeLevel {
        self.level
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }

    fn is_expired_at(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.raised_at) >= NOTICE_LIFETIME
    }

    fn is_routed_to(&self, open_request: Option<RequestTag>) -> bool {
        self.origin.is_some() && self.origin == open_request
    }
}

/// Every notice still on screen, oldest first. A new notice never replaces
/// an older one: each surface draws the newest notice meant for it.
#[derive(Debug, Default)]
pub struct NoticeQueue {
    notices: VecDeque<Notice>,
}

impl NoticeQueue {
    pub fn push(&mut self, notice: Notice) {
        if self.notices.len() == MAX_QUEUED_NOTICES {
            self.notices.pop_front();
        }
        self.notices.push_back(notice);
    }

    pub fn expire(&mut self, now: Instant) {
        self.notices.retain(|notice| !notice.is_expired_at(now));
    }

    pub fn dismiss(&mut self, level: NoticeLevel) {
        self.notices.retain(|notice| notice.level != level);
    }

    /// The newest notice of `level` that the open modal does not show itself.
    pub fn latest_outside(
        &self,
        level: NoticeLevel,
        open_request: Option<RequestTag>,
    ) -> Option<&Notice> {
        self.notices
            .iter()
            .rev()
            .find(|notice| notice.level == level && !notice.is_routed_to(open_request))
    }

    /// The newest notice, of any level, about the request `tag`.
    pub fn latest_for(&self, tag: RequestTag) -> Option<&Notice> {
        self.notices
            .iter()
            .rev()
            .find(|notice| notice.origin == Some(tag))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(queue: &NoticeQueue) -> Vec<&str> {
        queue.notices.iter().map(Notice::text).collect()
    }

    #[test]
    fn newer_error_is_shown_without_discarding_the_older_one() {
        let mut queue = NoticeQueue::default();
        queue.push(Notice::error("first"));
        queue.push(Notice::error("second"));

        assert_eq!(
            queue
                .latest_outside(NoticeLevel::Error, None)
                .map(Notice::text),
            Some("second")
        );
        assert_eq!(texts(&queue), ["first", "second"]);
    }

    #[test]
    fn errors_and_infos_are_looked_up_separately() {
        let mut queue = NoticeQueue::default();
        queue.push(Notice::info("Selected all 3 items"));
        queue.push(Notice::error("push rejected"));

        assert_eq!(
            queue
                .latest_outside(NoticeLevel::Info, None)
                .map(Notice::text),
            Some("Selected all 3 items")
        );
        assert_eq!(
            queue
                .latest_outside(NoticeLevel::Error, None)
                .map(Notice::text),
            Some("push rejected")
        );
    }

    #[test]
    fn notice_of_the_open_modal_request_stays_out_of_the_toast() {
        let tag = RequestTag::default().next();
        let mut queue = NoticeQueue::default();
        queue.push(Notice::error("toast error"));
        queue.push(Notice::error("labels failed").from_request(tag));

        assert_eq!(
            queue
                .latest_outside(NoticeLevel::Error, Some(tag))
                .map(Notice::text),
            Some("toast error")
        );
        assert_eq!(
            queue.latest_for(tag).map(Notice::text),
            Some("labels failed")
        );
    }

    #[test]
    fn notice_of_a_closed_modal_falls_back_to_the_toast() {
        let closed = RequestTag::default().next();
        let open = closed.next();
        let mut queue = NoticeQueue::default();
        queue.push(Notice::error("labels failed").from_request(closed));

        assert_eq!(
            queue
                .latest_outside(NoticeLevel::Error, Some(open))
                .map(Notice::text),
            Some("labels failed")
        );
        assert_eq!(
            queue
                .latest_outside(NoticeLevel::Error, None)
                .map(Notice::text),
            Some("labels failed")
        );
    }

    #[test]
    fn expire_drops_notices_once_their_lifetime_passed() {
        let mut queue = NoticeQueue::default();
        queue.push(Notice::error("old"));
        let pushed = queue.notices[0].raised_at;

        queue.expire(pushed + NOTICE_LIFETIME - Duration::from_millis(1));
        assert_eq!(texts(&queue), ["old"]);

        queue.expire(pushed + NOTICE_LIFETIME);
        assert!(queue.notices.is_empty());
    }

    #[test]
    fn dismiss_keeps_notices_of_the_other_level() {
        let mut queue = NoticeQueue::default();
        queue.push(Notice::info("Loading the stack of PR #4…"));
        queue.push(Notice::error("push rejected"));

        queue.dismiss(NoticeLevel::Info);

        assert_eq!(texts(&queue), ["push rejected"]);
    }

    #[test]
    fn queue_drops_its_oldest_notice_past_the_cap() {
        let mut queue = NoticeQueue::default();
        for i in 0..=MAX_QUEUED_NOTICES {
            queue.push(Notice::info(i.to_string()));
        }

        assert_eq!(queue.notices.len(), MAX_QUEUED_NOTICES);
        assert_eq!(queue.notices.front().map(Notice::text), Some("1"));
    }
}
