//! Cancellation-safe Web API admission: paced starts, shared cooldown and foreground priority.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::Instant;

const MAX_IN_FLIGHT: usize = 6;
const REQUEST_SPACING: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Priority {
    Foreground,
    Background,
}

tokio::task_local! {
    static REQUEST_PRIORITY: Priority;
}

/// Propagate the request owner's priority through gateway fallbacks and paging.
pub(crate) async fn with_priority<F: std::future::Future>(
    priority: Priority,
    request: F,
) -> F::Output {
    REQUEST_PRIORITY.scope(priority, request).await
}

pub(crate) fn current_priority() -> Priority {
    REQUEST_PRIORITY
        .try_with(|priority| *priority)
        .unwrap_or(Priority::Foreground)
}

struct State {
    next_ticket: u64,
    foreground: VecDeque<u64>,
    background: VecDeque<u64>,
    active: usize,
    next_request_at: Instant,
    cooldown_until: Instant,
}

pub(crate) struct RequestGate {
    state: Mutex<State>,
    changed: Notify,
}

impl RequestGate {
    pub(crate) fn new() -> Arc<Self> {
        let now = Instant::now();
        Arc::new(Self {
            state: Mutex::new(State {
                next_ticket: 0,
                foreground: VecDeque::new(),
                background: VecDeque::new(),
                active: 0,
                next_request_at: now,
                cooldown_until: now,
            }),
            changed: Notify::new(),
        })
    }

    pub(crate) fn extend_cooldown(&self, wait: Duration) {
        let now = Instant::now();
        let requested = now
            .checked_add(wait)
            .unwrap_or_else(|| now + Duration::from_secs(u32::MAX.into()));
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.cooldown_until = state.cooldown_until.max(requested);
        drop(state);
        self.changed.notify_waiters();
    }

    #[cfg(test)]
    pub(crate) fn cooling_down(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cooldown_until
            > Instant::now()
    }

    pub(crate) async fn acquire(self: &Arc<Self>, priority: Priority) -> RequestPermit {
        let ticket = {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            let ticket = state.next_ticket;
            state.next_ticket = state.next_ticket.wrapping_add(1);
            match priority {
                Priority::Foreground => &mut state.foreground,
                Priority::Background => &mut state.background,
            }
            .push_back(ticket);
            ticket
        };
        let mut waiting = Waiting {
            gate: Arc::clone(self),
            ticket,
            priority,
            queued: true,
        };
        self.changed.notify_waiters();
        loop {
            // Register before checking state: a completion or cancellation
            // between the check and the await must not lose its wakeup.
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let due = {
                let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
                let now = Instant::now();
                let due = state.next_request_at.max(state.cooldown_until);
                // Foreground work takes the next eligible start. Each class stays FIFO.
                let first = state
                    .foreground
                    .front()
                    .or_else(|| state.background.front())
                    .copied();
                if first == Some(ticket) && state.active < MAX_IN_FLIGHT && due <= now {
                    match priority {
                        Priority::Foreground => &mut state.foreground,
                        Priority::Background => &mut state.background,
                    }
                    .pop_front();
                    waiting.queued = false;
                    state.active += 1;
                    state.next_request_at = now + REQUEST_SPACING;
                    drop(state);
                    self.changed.notify_waiters();
                    return RequestPermit(Arc::clone(self));
                }
                (first == Some(ticket) && state.active < MAX_IN_FLIGHT && due > now).then_some(due)
            };
            if let Some(due) = due {
                tokio::select! { _ = &mut changed => {}, _ = tokio::time::sleep_until(due) => {} }
            } else {
                changed.await;
            }
        }
    }
}

struct Waiting {
    gate: Arc<RequestGate>,
    ticket: u64,
    priority: Priority,
    queued: bool,
}
impl Drop for Waiting {
    fn drop(&mut self) {
        if self.queued {
            let mut state = self.gate.state.lock().unwrap_or_else(|p| p.into_inner());
            match self.priority {
                Priority::Foreground => &mut state.foreground,
                Priority::Background => &mut state.background,
            }
            .retain(|ticket| *ticket != self.ticket);
            drop(state);
            self.gate.changed.notify_waiters();
        }
    }
}

pub(crate) struct RequestPermit(Arc<RequestGate>);
impl Drop for RequestPermit {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|p| p.into_inner());
        state.active -= 1;
        drop(state);
        self.0.changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn queued(
        gate: &Arc<RequestGate>,
        priority: Priority,
        label: u8,
        sent: tokio::sync::mpsc::UnboundedSender<(u8, Instant, RequestPermit)>,
    ) -> tokio::task::JoinHandle<()> {
        let gate = Arc::clone(gate);
        let task = tokio::spawn(async move {
            let permit = gate.acquire(priority).await;
            sent.send((label, Instant::now(), permit)).unwrap();
        });
        tokio::task::yield_now().await;
        task
    }

    #[tokio::test]
    async fn request_priority_is_scoped_and_unrelated_tasks_default_to_foreground() {
        assert_eq!(current_priority(), Priority::Foreground);
        with_priority(Priority::Background, async {
            assert_eq!(current_priority(), Priority::Background);
            with_priority(Priority::Foreground, async {
                assert_eq!(current_priority(), Priority::Foreground);
            })
            .await;
            assert_eq!(current_priority(), Priority::Background);
            assert_eq!(
                tokio::spawn(async { current_priority() }).await.unwrap(),
                Priority::Foreground
            );
        })
        .await;
        assert_eq!(current_priority(), Priority::Foreground);
    }

    #[tokio::test(start_paused = true)]
    async fn foreground_gets_next_eligible_slot_ahead_of_background() {
        let gate = RequestGate::new();
        gate.extend_cooldown(Duration::from_secs(120));
        let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
        queued(&gate, Priority::Background, 1, sent.clone()).await;
        queued(&gate, Priority::Background, 2, sent.clone()).await;
        queued(&gate, Priority::Foreground, 3, sent.clone()).await;
        queued(&gate, Priority::Foreground, 4, sent).await;
        tokio::time::advance(Duration::from_secs(119)).await;
        tokio::task::yield_now().await;
        assert!(received.try_recv().is_err());
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        let first = received.try_recv().unwrap();
        assert_eq!(first.0, 3, "foreground must take the next eligible slot");
        assert!(received.try_recv().is_err());
        for expected in [4, 1, 2] {
            tokio::time::advance(REQUEST_SPACING).await;
            tokio::task::yield_now().await;
            assert_eq!(received.try_recv().unwrap().0, expected);
            assert!(received.try_recv().is_err());
        }
    }

    #[tokio::test(start_paused = true)]
    async fn cancellation_releases_queued_priority_and_active_capacity() {
        let gate = RequestGate::new();
        gate.extend_cooldown(Duration::from_secs(60));
        let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
        let cancelled = queued(&gate, Priority::Foreground, 1, sent.clone()).await;
        queued(&gate, Priority::Background, 2, sent).await;
        cancelled.abort();
        assert!(cancelled.await.unwrap_err().is_cancelled());
        tokio::time::advance(Duration::from_secs(60)).await;
        tokio::task::yield_now().await;
        assert_eq!(received.try_recv().unwrap().0, 2);
        assert_eq!(gate.state.lock().unwrap().active, 0);
        assert!(gate.state.lock().unwrap().foreground.is_empty());
        assert!(gate.state.lock().unwrap().background.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn waits_own_no_capacity_and_active_requests_remain_bounded() {
        let gate = RequestGate::new();
        let mut held = Vec::new();
        for _ in 0..MAX_IN_FLIGHT {
            held.push(gate.acquire(Priority::Foreground).await);
        }
        let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
        queued(&gate, Priority::Background, 1, sent.clone()).await;
        queued(&gate, Priority::Foreground, 2, sent).await;
        tokio::time::advance(REQUEST_SPACING).await;
        tokio::task::yield_now().await;
        assert!(received.try_recv().is_err());
        drop(held.pop());
        tokio::task::yield_now().await;
        let admitted = received.try_recv().unwrap();
        assert_eq!(admitted.0, 2);
        assert_eq!(gate.state.lock().unwrap().active, MAX_IN_FLIGHT);
        drop(admitted);
        gate.extend_cooldown(Duration::from_secs(120));
        assert_eq!(gate.state.lock().unwrap().active, MAX_IN_FLIGHT - 1);
        tokio::time::advance(Duration::from_secs(119)).await;
        tokio::task::yield_now().await;
        assert!(received.try_recv().is_err());
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(received.try_recv().unwrap().0, 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_later_cooldown_extension_is_rechecked_and_never_shortened() {
        let gate = RequestGate::new();
        gate.extend_cooldown(Duration::from_secs(60));
        let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
        queued(&gate, Priority::Foreground, 1, sent).await;
        tokio::time::advance(Duration::from_secs(30)).await;
        gate.extend_cooldown(Duration::from_secs(120));
        gate.extend_cooldown(Duration::from_secs(1));
        tokio::time::advance(Duration::from_secs(119)).await;
        tokio::task::yield_now().await;
        assert!(gate.cooling_down());
        assert!(received.try_recv().is_err());
        tokio::time::advance(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert!(!gate.cooling_down());
        assert_eq!(received.try_recv().unwrap().0, 1);
    }
}
