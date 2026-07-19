#[derive(Debug, Clone)]
pub enum SupervisorStrategy {
    OneForOne {
        max_restarts: u32,
        within_secs: u64,
    },
    AllForOne {
        max_restarts: u32,
        within_secs: u64,
    },
    /// Restart up to `max` times within `within_secs`. Use with `spawn_with_factory` / `spawn_child_with_factory`.
    RestartN {
        max: usize,
        within_secs: u64,
    },
    Escalate,
}

impl Default for SupervisorStrategy {
    fn default() -> Self {
        SupervisorStrategy::OneForOne {
            max_restarts: 3,
            within_secs: 60,
        }
    }
}

/// Returns `true` if a restart is allowed (and records it), `false` if limit exceeded.
/// Uses `tokio::time::Instant` so `tokio::time::pause()/advance()` works in tests.
pub fn check_restart_limit(
    restart_times: &mut Vec<tokio::time::Instant>,
    max_restarts: u32,
    within_secs: u64,
) -> bool {
    let now = tokio::time::Instant::now();
    let window = std::time::Duration::from_secs(within_secs);
    restart_times.retain(|&t| now.duration_since(t) < window);
    if restart_times.len() < max_restarts as usize {
        restart_times.push(now);
        true
    } else {
        false
    }
}
