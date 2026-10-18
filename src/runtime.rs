use std::time::{Duration, Instant};

use crate::{config::Rule, protection::Options, recovery::Identity, window::WindowInfo};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FailureKind {
    Transient,
    Permanent,
    Gone,
}

pub struct Attempt {
    pub target: WindowInfo,
    pub rule: Rule,
    pub error: Option<String>,
    retries: usize,
    next: Option<Instant>,
}

impl Attempt {
    pub fn failed(
        target: WindowInfo,
        rule: Rule,
        error: String,
        kind: FailureKind,
        now: Instant,
    ) -> Self {
        Self {
            target,
            rule,
            error: Some(error),
            retries: 0,
            next: (kind == FailureKind::Transient).then_some(now + Duration::from_secs(1)),
        }
    }

    pub fn due(&self, now: Instant) -> bool {
        self.next.is_some_and(|next| now >= next)
    }

    pub fn fail_again(&mut self, error: String, kind: FailureKind, now: Instant) {
        self.error = Some(error);
        self.retries += 1;
        self.next = if kind == FailureKind::Transient {
            [1, 3, 10]
                .get(self.retries)
                .map(|seconds| now + Duration::from_secs(*seconds))
        } else {
            None
        };
    }

    pub fn waiting(&self) -> bool {
        self.next.is_some()
    }
}

pub struct Release {
    pub target: WindowInfo,
    pub identity: Identity,
    pub options: Options,
    pub deadline: Instant,
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::HWND;

    #[test]
    fn retry_delays_are_bounded_and_permanent_failures_stop() {
        let now = Instant::now();
        let target = WindowInfo {
            hwnd: HWND::default(),
            pid: 1,
            thread_id: 1,
            title: String::new(),
            class: "Test".into(),
        };
        let rule = Rule {
            name: String::new(),
            path: "test.exe".into(),
            class: "Test".into(),
            title_contains: String::new(),
            automatic: true,
            options: Options::default(),
        };
        let mut attempt = Attempt::failed(target, rule, "busy".into(), FailureKind::Transient, now);
        for seconds in [1, 3, 10] {
            assert!(!attempt.due(now + Duration::from_millis(seconds * 1000 - 1)));
            assert!(attempt.due(now + Duration::from_secs(seconds)));
            attempt.fail_again("busy".into(), FailureKind::Transient, now);
        }
        assert!(!attempt.waiting());
        attempt.fail_again("denied".into(), FailureKind::Permanent, now);
        assert!(!attempt.waiting());
    }
}
