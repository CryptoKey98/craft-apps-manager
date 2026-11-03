use std::collections::{BTreeMap, BTreeSet};

pub(super) const DURATION: f32 = 0.26;
const DEPARTURE_DURATION: f64 = 0.14;

#[derive(Clone, Default)]
pub(super) struct Visibility {
    previous: Option<BTreeSet<String>>,
    started: BTreeMap<String, (bool, f64, f32)>,
}

impl Visibility {
    pub(super) fn update(
        &mut self,
        visible: BTreeSet<String>,
        ready: bool,
        now: f64,
        enabled: bool,
    ) {
        if !ready {
            return;
        }
        if enabled {
            if let Some(previous) = self.previous.clone() {
                for (name, showing) in visible
                    .difference(&previous)
                    .map(|name| (name, true))
                    .chain(previous.difference(&visible).map(|name| (name, false)))
                {
                    let from = if self.started.contains_key(name) {
                        self.opacity(name, now)
                    } else if showing {
                        0.0
                    } else {
                        1.0
                    };
                    self.started.insert(name.clone(), (showing, now, from));
                }
            }
        } else {
            self.started.clear();
        }
        self.started.retain(|name, (showing, start, _)| {
            visible.contains(name) == *showing
                && now - *start
                    < if *showing {
                        f64::from(DURATION)
                    } else {
                        DEPARTURE_DURATION + f64::from(DURATION)
                    }
        });
        self.previous = Some(visible);
    }

    pub(super) fn visible(&self, name: &str, actual: bool, now: f64) -> bool {
        actual
            || self
                .started
                .get(name)
                .is_some_and(|(showing, start, _)| !showing && now - start < DEPARTURE_DURATION)
    }

    pub(super) fn opacity(&self, name: &str, now: f64) -> f32 {
        self.started
            .get(name)
            .map_or(1.0, |(showing, start, from)| {
                if *showing {
                    from + (1.0 - from) * eased(now - start)
                } else {
                    let t = ((now - start) / DEPARTURE_DURATION).clamp(0.0, 1.0) as f32;
                    from * (1.0 - t * t * (3.0 - 2.0 * t))
                }
            })
    }

    pub(super) fn moving(&self, name: &str) -> bool {
        self.started.contains_key(name)
    }

    pub(super) fn active_matching(&self, matches: impl Fn(&str) -> bool) -> bool {
        self.started.keys().any(|name| matches(name))
    }
}

#[derive(Clone, Default)]
pub(super) struct Arrivals {
    previous: Option<BTreeSet<String>>,
    started: BTreeMap<String, (bool, f64)>,
    first_section: Option<f64>,
    last_section: Option<(f64, usize)>,
}

impl Arrivals {
    pub(super) fn update(
        &mut self,
        installed: BTreeSet<String>,
        ready: bool,
        now: f64,
        enabled: bool,
    ) {
        if !ready {
            return;
        }
        if enabled {
            if let Some(previous) = self.previous.clone() {
                for (name, destination) in installed
                    .difference(&previous)
                    .map(|name| (name, true))
                    .chain(previous.difference(&installed).map(|name| (name, false)))
                {
                    if self.displayed_installed(name, previous.contains(name), now) == destination {
                        self.started.remove(name);
                    } else {
                        self.started.insert(name.clone(), (destination, now));
                    }
                }
                if previous.is_empty() && !installed.is_empty() {
                    self.first_section = Some(now + DEPARTURE_DURATION);
                    self.last_section = None;
                } else if !previous.is_empty() && installed.is_empty() {
                    self.last_section = Some((now + DEPARTURE_DURATION, previous.len()));
                    self.first_section = None;
                }
            }
        } else {
            self.started.clear();
            self.first_section = None;
            self.last_section = None;
        }
        self.started.retain(|name, (destination, start)| {
            installed.contains(name) == *destination
                && now - *start < DEPARTURE_DURATION + f64::from(DURATION)
        });
        if self.started.is_empty() {
            self.first_section = None;
            self.last_section = None;
        }
        if self
            .first_section
            .is_some_and(|start| now - start >= f64::from(DURATION))
        {
            self.first_section = None;
        }
        if self
            .last_section
            .is_some_and(|(start, _)| now - start >= f64::from(DURATION))
        {
            self.last_section = None;
        }
        self.previous = Some(installed);
    }

    pub(super) fn progress(&self, name: &str, now: f64) -> f32 {
        self.started
            .get(name)
            .map_or(1.0, |(_, start)| eased(now - start - DEPARTURE_DURATION))
    }

    pub(super) fn displayed_installed(&self, name: &str, actual: bool, now: f64) -> bool {
        self.started
            .get(name)
            .map_or(actual, |(destination, start)| {
                if now - start < DEPARTURE_DURATION {
                    !destination
                } else {
                    *destination
                }
            })
    }

    pub(super) fn moving(&self, name: &str) -> bool {
        self.started.contains_key(name)
    }

    pub(super) fn departing(&self, name: &str, now: f64) -> bool {
        self.started
            .get(name)
            .is_some_and(|(_, start)| now - start < DEPARTURE_DURATION)
    }

    pub(super) fn opacity(&self, name: &str, now: f64) -> f32 {
        if let Some((_, start)) = self.started.get(name) {
            let elapsed = now - start;
            if elapsed < DEPARTURE_DURATION {
                let t = (elapsed / DEPARTURE_DURATION).clamp(0.0, 1.0) as f32;
                return 1.0 - t * t * (3.0 - 2.0 * t);
            }
        }
        self.progress(name, now)
    }

    pub(super) fn header_progress(&self, now: f64) -> f32 {
        self.last_section.map_or_else(
            || self.first_section.map_or(1.0, |start| eased(now - start)),
            |(start, _)| 1.0 - eased(now - start),
        )
    }

    pub(super) fn closing_count(&self) -> Option<usize> {
        self.last_section.map(|(_, count)| count)
    }

    pub(super) fn active(&self) -> bool {
        !self.started.is_empty()
    }
}

fn eased(elapsed: f64) -> f32 {
    let t = (elapsed / f64::from(DURATION)).clamp(0.0, 1.0) as f32;
    1.0 - (1.0 - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hide_fades_before_removal_and_unhide_fades_in_without_startup_replay() {
        let mut visibility = Visibility::default();
        visibility.update(names(&["filmcraft"]), true, 0.0, true);
        assert!(!visibility.active_matching(|_| true));
        visibility.update(names(&[]), true, 1.0, true);
        assert!(visibility.visible("filmcraft", false, 1.0));
        assert_eq!(visibility.opacity("filmcraft", 1.0), 1.0);
        assert!(visibility.opacity("filmcraft", 1.07) < 1.0);
        assert!(!visibility.visible("filmcraft", false, 1.15));
        assert!(
            visibility.active_matching(|_| true),
            "leave time for the vacated gap to close"
        );
        visibility.update(names(&[]), true, 1.6, true);
        assert!(!visibility.active_matching(|_| true));
        visibility.update(names(&["filmcraft"]), true, 2.0, true);
        assert!(visibility.visible("filmcraft", true, 2.0));
        assert_eq!(visibility.opacity("filmcraft", 2.0), 0.0);
        assert!(visibility.opacity("filmcraft", 2.13) > 0.5);
        visibility.update(names(&[]), true, 2.13, true);
        assert!(
            visibility.opacity("filmcraft", 2.13) > 0.5,
            "reversing a partial fade must not jump"
        );
        visibility.update(names(&[]), true, 2.2, false);
        assert!(!visibility.visible("filmcraft", false, 2.2));
        assert!(!visibility.active_matching(|_| true));
    }
    #[test]
    fn both_directions_fade_out_before_changing_sections() {
        for initially_installed in [false, true] {
            let mut motion = Arrivals::default();
            motion.update(
                if initially_installed {
                    names(&["filmcraft"])
                } else {
                    names(&[])
                },
                true,
                0.0,
                true,
            );
            motion.update(
                if initially_installed {
                    names(&[])
                } else {
                    names(&["filmcraft"])
                },
                true,
                1.0,
                true,
            );
            assert_eq!(
                motion.displayed_installed("filmcraft", !initially_installed, 1.0),
                initially_installed
            );
            assert_eq!(motion.opacity("filmcraft", 1.0), 1.0);
            assert!(motion.opacity("filmcraft", 1.07) < 1.0);
            assert_eq!(
                motion.displayed_installed("filmcraft", !initially_installed, 1.15),
                !initially_installed
            );
            assert!(motion.opacity("filmcraft", 1.15) < 0.2);
            assert_eq!(motion.opacity("filmcraft", 1.5), 1.0);
        }
    }
    #[test]
    fn uninstall_fades_into_available_and_collapses_the_last_installed_section() {
        let mut motion = Arrivals::default();
        motion.update(names(&["filmcraft"]), true, 0.0, true);
        motion.update(names(&[]), true, 1.0, true);
        assert!(motion.active());
        assert_eq!(motion.progress("filmcraft", 1.0), 0.0);
        assert_eq!(motion.header_progress(1.0), 1.0);
        assert!(motion.header_progress(1.27) < 0.5);
        motion.update(names(&[]), true, 1.5, true);
        assert!(!motion.active());
    }
    fn names(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn initial_inventory_does_not_animate_but_first_install_does() {
        let mut motion = Arrivals::default();
        motion.update(names(&[]), false, 0.0, true);
        motion.update(names(&[]), true, 1.0, true);
        motion.update(names(&["filmcraft"]), true, 2.0, true);
        assert_eq!(motion.progress("filmcraft", 2.0), 0.0);
        assert_eq!(motion.header_progress(2.0), 0.0);
        assert!(motion.progress("filmcraft", 2.27) > 0.5);
        motion.update(names(&["filmcraft"]), true, 2.5, true);
        assert!(!motion.active());
        assert_eq!(motion.progress("filmcraft", 2.4), 1.0);
        let mut reopened = Arrivals::default();
        reopened.update(names(&["filmcraft"]), true, 1.0, true);
        assert!(!reopened.active());
    }

    #[test]
    fn additional_and_batch_installs_only_fade_new_rows() {
        let mut motion = Arrivals::default();
        motion.update(names(&["soundcraft"]), true, 0.0, true);
        motion.update(
            names(&["soundcraft", "filmcraft", "photocraft"]),
            true,
            1.0,
            true,
        );
        assert_eq!(motion.progress("soundcraft", 1.0), 1.0);
        assert_eq!(motion.header_progress(1.0), 1.0);
        assert_eq!(motion.progress("filmcraft", 1.0), 0.0);
        assert_eq!(motion.progress("photocraft", 1.0), 0.0);
        motion.update(names(&["soundcraft"]), true, 1.1, true);
        assert!(
            !motion.active(),
            "reversal before departure finishes keeps the original rows visible"
        );
        assert_eq!(motion.progress("filmcraft", 1.1), 1.0);
        motion.update(names(&["soundcraft"]), true, 1.5, true);
        assert!(!motion.active());
    }

    #[test]
    fn disabled_motion_and_unchanged_installation_do_not_replay() {
        let mut motion = Arrivals::default();
        motion.update(names(&[]), true, 0.0, true);
        motion.update(names(&["filmcraft"]), true, 1.0, false);
        assert!(!motion.active());
        motion.update(names(&["filmcraft"]), true, 2.0, true);
        assert!(!motion.active());
    }
}
