//! Popup intent and input ownership. No clocks, sleeps or frontend focus policy.

#[derive(Clone, Copy, Debug, Default)]
pub struct FocusContext {
    pub focused: bool,
    pub visible: bool,
    pub over_popup: bool,
    pub over_tray: bool,
    pub button_down: bool,
    pub tray_host_focused: bool,
    pub same_process: bool,
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(windows), allow(dead_code))] // Native Windows gestures remain portable unit-test inputs.
pub enum Input {
    Show,
    Hide,
    Toggle,
    Pin,
    TogglePin,
    TrayDown,
    TrayUp,
    TrayDouble,
    TrayMenuDown,
    TrayLeft(FocusContext),
    #[cfg_attr(windows, allow(dead_code))]
    PlatformDown,
    #[cfg_attr(windows, allow(dead_code))]
    PlatformUp,
    MenuOpened,
    MenuClosed(FocusContext),
    Focus(FocusContext),
    Shortcut(bool),
    Management(bool),
    Ready,
    Destroyed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Effects {
    pub show: bool,
    pub hide: bool,
    pub focus: bool,
    pub opened: bool,
    pub pin: Option<bool>,
    pub create: bool,
}

#[derive(Debug, Default)]
pub struct Popup {
    pub visible: bool,
    pub pinned: bool,
    ready: bool,
    tray_down: bool,
    tray_menu: bool,
    menu_open: bool,
    shortcut_down: bool,
    management: bool,
    release_target: Option<bool>,
    release_cancelled: bool,
}

impl Popup {
    pub const fn new() -> Self {
        Self {
            visible: false,
            pinned: false,
            ready: false,
            tray_down: false,
            tray_menu: false,
            menu_open: false,
            shortcut_down: false,
            management: false,
            release_target: None,
            release_cancelled: false,
        }
    }

    fn clear_tray(&mut self) {
        self.release_cancelled |= self.release_target.take().is_some();
        self.tray_down = false;
        self.tray_menu = false;
    }

    fn set_visible(&mut self, visible: bool, effects: &mut Effects) {
        let changed = self.visible != visible;
        self.visible = visible;
        if visible {
            effects.create = !self.ready;
            effects.show = self.ready;
            effects.focus = self.ready;
            effects.opened = changed && self.ready;
        } else {
            effects.hide = self.ready;
            self.pinned = false;
        }
    }

    fn blur(&mut self, context: FocusContext, effects: &mut Effects) {
        if context.focused
            || !self.visible
            || self.pinned
            || self.management
            || self.menu_open
            || context.over_popup
            || self.release_target.is_some()
        {
            return;
        }
        // Windows can send focus loss before its tray callback. Keep the
        // pre-press intent until that *same physical gesture* reaches us.
        if context.over_tray
            && (context.button_down
                || context.tray_host_focused
                || self.tray_down
                || self.tray_menu)
        {
            return;
        }
        self.set_visible(false, effects);
    }

    pub fn apply(&mut self, input: Input) -> Effects {
        let was_pinned = self.pinned;
        let mut effects = Effects::default();
        match input {
            Input::Show => {
                self.clear_tray();
                self.set_visible(true, &mut effects);
            }
            Input::Hide => {
                self.clear_tray();
                self.set_visible(false, &mut effects);
            }
            Input::Toggle => {
                self.clear_tray();
                self.set_visible(!self.visible, &mut effects);
            }
            Input::Pin => {
                self.clear_tray();
                self.pinned = true;
                self.set_visible(true, &mut effects);
            }
            Input::TogglePin => self.pinned = !self.pinned,
            Input::TrayDown if !self.tray_down => {
                self.tray_down = true;
                self.set_visible(!self.visible, &mut effects);
            }
            Input::TrayUp => {
                effects.focus = self.tray_down && self.visible && self.ready && !self.management;
                self.tray_down = false;
            }
            // Windows emits DoubleClick in place of the second Down. Its
            // first press already made the decision: never toggle it twice.
            Input::TrayDouble => self.tray_down = true,
            Input::TrayMenuDown => self.tray_menu = true,
            Input::TrayLeft(context) => {
                if !context.over_tray && !self.menu_open {
                    self.clear_tray();
                    self.blur(context, &mut effects);
                }
            }
            Input::PlatformDown => {
                self.release_cancelled = false;
                self.release_target = Some(!self.visible);
                self.tray_down = true;
            }
            Input::PlatformUp => {
                self.tray_down = false;
                if self.release_cancelled {
                    self.release_cancelled = false;
                } else {
                    let target = self.release_target.take().unwrap_or(!self.visible);
                    self.set_visible(target, &mut effects);
                }
            }
            Input::MenuOpened => self.menu_open = true,
            Input::MenuClosed(context) => {
                self.menu_open = false;
                self.tray_menu = false;
                if self.visible && !self.management {
                    if context.same_process || context.over_popup {
                        effects.focus = self.ready;
                    } else {
                        self.blur(context, &mut effects);
                    }
                }
            }
            Input::Focus(context) => {
                if !self.ready {
                    return effects;
                }
                if context.focused {
                    // Also observe explicit native window activation (e.g.
                    // accessibility), without making the WebView an owner.
                    self.visible = context.visible;
                } else if !context.visible && self.ready {
                    self.visible = false;
                } else {
                    self.blur(context, &mut effects);
                }
            }
            Input::Shortcut(pressed) => {
                if pressed && !self.shortcut_down {
                    self.shortcut_down = true;
                    return self.apply(Input::Toggle);
                }
                self.shortcut_down = pressed;
            }
            Input::Management(open) => {
                self.management = open;
                self.clear_tray();
            }
            Input::Ready => {
                self.ready = true;
                effects.show = self.visible;
                effects.hide = !self.visible;
                effects.focus = self.visible;
                effects.opened = self.visible;
                effects.pin = Some(self.pinned);
            }
            Input::Destroyed => {
                self.ready = false;
                self.visible = false;
                self.pinned = false;
                self.clear_tray();
            }
            Input::TrayDown => {}
        }
        if was_pinned != self.pinned {
            effects.pin = Some(self.pinned);
        }
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn visible() -> Popup {
        let mut p = Popup::new();
        p.apply(Input::Show);
        p.apply(Input::Ready);
        p
    }
    fn outside() -> FocusContext {
        FocusContext {
            visible: true,
            ..Default::default()
        }
    }
    fn tray() -> FocusContext {
        FocusContext {
            over_tray: true,
            button_down: true,
            ..outside()
        }
    }

    #[test]
    fn blur_before_down_and_release_after_arbitrarily_long_hold_never_reopen() {
        let mut p = visible();
        assert_eq!(p.apply(Input::Focus(tray())), Effects::default());
        assert!(p.apply(Input::TrayDown).hide);
        for _ in 0..100 {
            p.apply(Input::Focus(tray()));
        }
        assert!(!p.apply(Input::TrayUp).show);
        assert!(!p.visible);
    }
    #[test]
    fn down_before_blur_hides_once_and_orphan_releases_are_inert() {
        let mut p = visible();
        assert!(p.apply(Input::TrayDown).hide);
        assert_eq!(p.apply(Input::Focus(outside())), Effects::default());
        assert_eq!(p.apply(Input::TrayUp), Effects::default());
        assert_eq!(p.apply(Input::TrayUp), Effects::default());
        assert!(!p.visible);
    }

    #[test]
    fn batched_double_click_can_release_physical_button_before_the_blur_callback() {
        let mut p = visible();
        let released_tray = FocusContext {
            button_down: false,
            tray_host_focused: true,
            ..tray()
        };
        assert_eq!(p.apply(Input::Focus(released_tray)), Effects::default());
        assert!(p.apply(Input::TrayDown).hide);
        p.apply(Input::TrayUp);
        p.apply(Input::TrayDouble);
        assert_eq!(p.apply(Input::TrayUp), Effects::default());
        assert!(!p.visible);
        let mut p = visible();
        assert!(
            p.apply(Input::Focus(FocusContext {
                over_tray: true,
                ..outside()
            }))
            .hide,
            "Hovering the tray must not suppress a genuine external focus loss"
        );
    }
    #[test]
    fn one_hundred_rounds_change_visibility_exactly_once_per_press() {
        let mut p = visible();
        for _ in 0..100 {
            p.apply(Input::Focus(tray()));
            assert!(p.apply(Input::TrayDown).hide);
            assert_eq!(p.apply(Input::TrayUp), Effects::default());
            assert!(!p.visible);
            assert!(p.apply(Input::TrayDown).show);
            p.apply(Input::Focus(tray()));
            assert!(p.apply(Input::TrayUp).focus);
            assert!(p.visible);
        }
    }
    #[test]
    fn native_double_click_is_one_toggle_in_both_initial_states() {
        for initial_visible in [false, true] {
            let mut p = visible();
            if !initial_visible {
                p.apply(Input::Hide);
            }
            p.apply(Input::TrayDown);
            p.apply(Input::TrayUp);
            p.apply(Input::Focus(tray()));
            assert_eq!(p.apply(Input::TrayDouble), Effects::default());
            let up = p.apply(Input::TrayUp);
            assert!(!up.show && !up.hide);
            assert_eq!(p.visible, !initial_visible);
        }
    }
    #[test]
    fn ordinary_blur_hides_but_pin_management_pointer_and_fresh_focus_do_not() {
        let mut p = visible();
        assert!(p.apply(Input::Focus(outside())).hide);
        for input in [Input::Pin, Input::Management(true)] {
            let mut p = visible();
            p.apply(input);
            assert!(!p.apply(Input::Focus(outside())).hide);
            assert!(p.visible);
        }
        for context in [
            FocusContext {
                over_popup: true,
                ..outside()
            },
            FocusContext {
                focused: true,
                ..outside()
            },
        ] {
            let mut p = visible();
            assert!(!p.apply(Input::Focus(context)).hide);
            assert!(p.visible);
        }
    }
    #[test]
    fn explicit_show_hide_and_pin_are_idempotent_and_override_inflight_gestures() {
        let mut p = visible();
        assert!(!p.apply(Input::Show).hide);
        assert!(p.visible);
        p.apply(Input::TrayDown);
        p.apply(Input::Show);
        assert_eq!(p.apply(Input::TrayUp), Effects::default());
        assert!(p.visible);
        p.apply(Input::Pin);
        p.apply(Input::Hide);
        assert!(!p.visible && !p.pinned);
        assert!(!p.apply(Input::Hide).show);
    }
    #[test]
    fn held_shortcut_and_late_tray_release_cannot_undo_a_new_intent() {
        let mut p = visible();
        p.apply(Input::TrayDown);
        assert!(p.apply(Input::Shortcut(true)).show);
        assert_eq!(p.apply(Input::Shortcut(true)), Effects::default());
        assert_eq!(p.apply(Input::TrayUp), Effects::default());
        assert!(p.visible);
        p.apply(Input::Shortcut(false));
        assert!(p.apply(Input::Shortcut(true)).hide);
    }
    #[test]
    fn pending_webview_ready_obeys_latest_intent_without_a_flash() {
        let mut p = Popup::new();
        assert!(p.apply(Input::Show).create);
        p.apply(Input::Focus(FocusContext::default()));
        assert!(p.visible);
        p.apply(Input::Hide);
        let ready = p.apply(Input::Ready);
        assert!(ready.hide && !ready.show && !ready.focus);
        assert!(!p.visible);
        p.apply(Input::Destroyed);
        assert!(p.apply(Input::Pin).create);
        let ready = p.apply(Input::Ready);
        assert!(ready.show && ready.focus);
        assert!(p.pinned);
    }
    #[test]
    fn context_menu_and_settings_do_not_hide_an_already_visible_popup() {
        let mut p = visible();
        p.apply(Input::Focus(tray()));
        p.apply(Input::TrayMenuDown);
        p.apply(Input::MenuOpened);
        assert!(!p.apply(Input::Focus(outside())).hide);
        p.apply(Input::Management(true));
        p.apply(Input::MenuClosed(outside()));
        assert!(p.visible);
        p.apply(Input::Management(false));
        assert!(p.apply(Input::Focus(outside())).hide);
    }
    #[test]
    fn cancelling_native_menu_restores_owned_focus_or_hides_for_an_external_target() {
        let mut p = visible();
        p.apply(Input::MenuOpened);
        assert!(
            p.apply(Input::MenuClosed(FocusContext {
                same_process: true,
                ..outside()
            }))
            .focus
        );
        assert!(p.visible);
        p.apply(Input::MenuOpened);
        assert!(p.apply(Input::MenuClosed(outside())).hide);
        assert!(!p.visible);
    }

    #[test]
    fn cancelled_drag_and_stale_leave_cannot_poison_the_next_click() {
        let mut p = visible();
        p.apply(Input::TrayDown);
        p.apply(Input::TrayLeft(outside()));
        assert_eq!(p.apply(Input::TrayUp), Effects::default());
        assert!(p.apply(Input::TrayDown).show);
        p.apply(Input::TrayLeft(tray()));
        assert!(p.apply(Input::TrayUp).focus);
    }

    #[test]
    fn other_platforms_keep_release_activation_and_new_intents_cancel_old_release() {
        let mut p = visible();
        assert_eq!(p.apply(Input::PlatformDown), Effects::default());
        assert_eq!(p.apply(Input::Focus(outside())), Effects::default());
        assert!(p.visible);
        assert!(p.apply(Input::PlatformUp).hide);
        assert!(
            p.apply(Input::PlatformUp).show,
            "Up-only platforms retain their toggle"
        );
        p.apply(Input::PlatformDown);
        p.apply(Input::Show);
        assert_eq!(p.apply(Input::PlatformUp), Effects::default());
        assert!(p.visible);
    }
}
