//! Rebuildable parser buffers, not attention/history/notification state.
//! A monotonic cold watermark prevents repeatedly parsing evicted unchanged
//! history. Native append/replacement updates modified/created timestamps and
//! re-enters discovery. Active and unresolved states are never budget-evicted.
use std::{
    collections::HashSet,
    path::PathBuf,
    time::{Duration, SystemTime},
};
pub const HORIZON: Duration = Duration::from_secs(7 * 86400);
pub const INACTIVE_ENTRIES: usize = 128;
pub const INACTIVE_BYTES: usize = 16 * 1024 * 1024;
#[derive(Default)]
pub struct Policy {
    cold_through: Option<SystemTime>,
}
impl Policy {
    /// WSL supplies Linux stat; do not mix its watermark with UNC times.
    pub fn eligible_observed(
        &self,
        modified: SystemTime,
        changed: SystemTime,
        protected: bool,
        cached: bool,
    ) -> bool {
        protected
            || cached
            || self
                .cold_through
                .is_none_or(|cut| modified > cut || changed > cut)
    }
    pub fn eligible(&self, path: &std::path::Path, protected: bool, cached: bool) -> bool {
        protected
            || cached
            || self.cold_through.is_none_or(|cut| {
                path.metadata().is_ok_and(|m| {
                    m.modified().is_ok_and(|at| at > cut) || m.created().is_ok_and(|at| at > cut)
                })
            })
    }
    pub fn keep(
        &mut self,
        items: Vec<(PathBuf, bool, usize)>,
        now: SystemTime,
    ) -> HashSet<PathBuf> {
        let items: Vec<_> = items
            .into_iter()
            .map(|(p, protected, bytes)| {
                let at = p
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or(SystemTime::UNIX_EPOCH);
                (p, protected, bytes, at)
            })
            .collect();
        self.keep_observed(items, now)
    }
    pub fn keep_observed(
        &mut self,
        mut items: Vec<(PathBuf, bool, usize, SystemTime)>,
        now: SystemTime,
    ) -> HashSet<PathBuf> {
        items.sort_by_key(|(_, _, _, at)| std::cmp::Reverse(*at));
        let mut keep = HashSet::new();
        let mut count = 0;
        let mut size = 0usize;
        for (path, protected, bytes, at) in items {
            if protected
                || count < INACTIVE_ENTRIES
                    && size.saturating_add(bytes) <= INACTIVE_BYTES
                    && now.duration_since(at).unwrap_or_default() <= HORIZON
            {
                keep.insert(path);
                if !protected {
                    count += 1;
                    size += bytes;
                }
            } else {
                self.cold_through = Some(self.cold_through.map_or(at, |old| old.max(at)));
            }
        }
        keep
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wsl_continuity_watermark_uses_linux_times_not_unc_drift() {
        let mut policy = Policy::default();
        let at = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        let path = PathBuf::from("not-an-actual-UNC-path");
        assert!(policy
            .keep_observed(
                vec![(path.clone(), false, 1, at)],
                at + HORIZON + Duration::from_secs(1)
            )
            .is_empty());
        for _ in 0..10 {
            assert!(!policy.eligible_observed(at, at, false, false));
        }
        assert!(policy.eligible_observed(at + Duration::from_secs(1), at, false, false));
        assert!(policy.eligible_observed(at, at, true, false));
    }
    #[test]
    fn budgets_preserve_active_and_do_not_reparse_evicted_unchanged_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut items = vec![];
        for n in 0..140 {
            let p = dir.path().join(n.to_string());
            std::fs::write(&p, b"{}").unwrap();
            items.push((p, n < 2, 1024));
        }
        let mut policy = Policy::default();
        let keep = policy.keep(items.clone(), SystemTime::now());
        assert_eq!(keep.len(), 130);
        assert!(keep.contains(&items[0].0));
        for (p, protected, _) in items {
            if !keep.contains(&p) {
                assert!(!policy.eligible(&p, protected, false));
            }
        }
    }
}
