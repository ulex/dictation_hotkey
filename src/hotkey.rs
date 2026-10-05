//! Portable parsing for the RegisterHotKey subset; reserved shortcuts need separate hooks.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: u32,
    pub vk: u32,
}
const ALT: u32 = 0x0001;
const CTRL: u32 = 0x0002;
const SHIFT: u32 = 0x0004;
const WIN: u32 = 0x0008;

pub fn parse(text: &str) -> Option<Hotkey> {
    let mut modifiers = 0;
    let mut key = None;
    for token in text.split('+').map(|t| t.trim().to_ascii_uppercase()) {
        let modifier = match token.as_str() {
            "CTRL" | "CONTROL" => CTRL,
            "ALT" => ALT,
            "SHIFT" => SHIFT,
            "WIN" | "WINDOWS" => WIN,
            _ => 0,
        };
        if modifier != 0 {
            if modifiers & modifier != 0 {
                return None;
            }
            modifiers |= modifier;
            continue;
        }
        if key.is_some() {
            return None;
        }
        key = match token.as_str() {
            s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphanumeric() => {
                Some(s.as_bytes()[0] as u32)
            }
            s if s.starts_with('F') => {
                let n: u32 = s[1..].parse().ok()?;
                if (1..=24).contains(&n) {
                    Some(0x70 + n - 1)
                } else {
                    None
                }
            }
            _ => None,
        };
        key?;
    }
    if modifiers == 0 {
        return None;
    }
    Some(Hotkey {
        modifiers,
        vk: key?,
    })
}

/// macOS uses Carbon physical key codes, with Command/Option aliases and F1–F20.
/// Keep Windows parser semantics unchanged for existing configurations.
pub fn valid_macos(text: &str) -> bool {
    let normalized = text
        .split('+')
        .map(|token| match token.trim().to_ascii_uppercase().as_str() {
            "CMD" | "COMMAND" => "WIN".to_owned(),
            "OPTION" => "ALT".to_owned(),
            "WIN" | "WINDOWS" => String::new(),
            other => other.to_owned(),
        })
        .collect::<Vec<_>>()
        .join("+");
    parse(&normalized)
        .is_some_and(|key| key.vk <= 0x83 && !(key.modifiers == WIN && key.vk == u32::from(b'V')))
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Action {
    Toggle,
    Stop,
    /// Replace this Win-key release with a masked, tagged release so the shell
    /// does not interpret a swallowed Win shortcut as a bare Win tap.
    MaskWindowsRelease,
}
/// Allocation-free key-repeat/suppressed-up tracking for the low-level hook.
pub struct Matcher {
    suppressed: [bool; 256],
    pressed: [bool; 256],
    escape: bool,
    mask_start: bool,
}
impl Default for Matcher {
    fn default() -> Self {
        Self {
            suppressed: [false; 256],
            pressed: [false; 256],
            escape: false,
            mask_start: false,
        }
    }
}
impl Matcher {
    /// Hook events are delivered before Windows updates asynchronous key state.
    /// Track both sides of every modifier rather than querying GetAsyncKeyState
    /// inside the hook (which is also unreliable for a burst of injected keys).
    pub fn modifiers(&self) -> u32 {
        let held = |keys: &[usize]| keys.iter().any(|&vk| self.pressed[vk]);
        u32::from(held(&[0x12, 0xa4, 0xa5]))
            | (u32::from(held(&[0x11, 0xa2, 0xa3])) << 1)
            | (u32::from(held(&[0x10, 0xa0, 0xa1])) << 2)
            | (u32::from(held(&[0x5b, 0x5c])) << 3)
    }
    #[allow(clippy::too_many_arguments)] // scalar-only hook inputs, no allocations
    pub fn event(
        &mut self,
        vk: u32,
        down: bool,
        own_injection: bool,
        modifiers: u32,
        win_h: bool,
        copilot: bool,
        recording: bool,
    ) -> (bool, Option<Action>) {
        if own_injection || vk >= 256 {
            return (false, None);
        }
        if vk == 0x1b {
            let action = if down && !self.escape && recording {
                Some(Action::Stop)
            } else {
                None
            };
            self.escape = down;
            return (false, action);
        }
        let was_down = self.pressed[vk as usize];
        self.pressed[vk as usize] = down;
        if !down && matches!(vk, 0x5b | 0x5c) && self.mask_start {
            self.mask_start = self.pressed[0x5b] || self.pressed[0x5c];
            return (true, Some(Action::MaskWindowsRelease));
        }
        let held = &mut self.suppressed[vk as usize];
        if *held {
            if !down {
                *held = false;
            }
            return (true, None);
        }
        let matches = (win_h && vk == 0x48 && modifiers == WIN)
            || (copilot
                && ((vk == 0x43 && modifiers == WIN) || (vk == 0x86 && modifiers == WIN | SHIFT)));
        if down && !was_down && matches {
            *held = true;
            self.mask_start = true;
            (true, Some(Action::Toggle))
        } else {
            (false, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn suppression_and_exact_modifiers() {
        let mut m = Matcher::default();
        assert_eq!(
            m.event(0x48, true, false, WIN | CTRL, true, false, false),
            (false, None)
        );
        // Releasing Ctrl while H is held must not arm a matching repeat.
        assert_eq!(
            m.event(0x48, true, false, WIN, true, false, false),
            (false, None)
        );
        assert_eq!(
            m.event(0x48, false, false, WIN, true, false, false),
            (false, None)
        );
        assert_eq!(
            m.event(0x48, true, false, WIN, true, false, false),
            (true, Some(Action::Toggle))
        );
        for _ in 0..100 {
            assert_eq!(
                m.event(0x48, true, false, WIN, true, false, true),
                (true, None)
            );
        }
        assert_eq!(
            m.event(0x48, false, false, 0, true, false, true),
            (true, None)
        );
        assert_eq!(
            m.event(0x48, true, true, WIN, true, false, true),
            (false, None)
        );
        assert_eq!(
            m.event(0x1b, true, false, 0, true, false, true),
            (false, Some(Action::Stop))
        );
        assert_eq!(
            m.event(0x1b, true, false, 0, true, false, true),
            (false, None)
        );
        assert_eq!(
            m.event(0x86, true, false, WIN | SHIFT, false, true, true),
            (true, Some(Action::Toggle))
        );
    }
    fn hook_event(m: &mut Matcher, vk: u32, down: bool) -> (bool, Option<Action>) {
        let modifiers = m.modifiers();
        m.event(vk, down, false, modifiers, true, true, false)
    }
    #[test]
    fn intercepted_chords_mask_both_win_keys_and_release_orders() {
        for win in [0x5b, 0x5c] {
            for win_first in [false, true] {
                let mut m = Matcher::default();
                assert_eq!(hook_event(&mut m, win, true), (false, None));
                assert_eq!(m.modifiers(), WIN);
                assert_eq!(hook_event(&mut m, 0x48, true), (true, Some(Action::Toggle)));
                assert_eq!(hook_event(&mut m, 0x48, true), (true, None));
                if !win_first {
                    assert_eq!(hook_event(&mut m, 0x48, false), (true, None));
                }
                assert_eq!(
                    hook_event(&mut m, win, false),
                    (true, Some(Action::MaskWindowsRelease))
                );
                assert_eq!(m.modifiers(), 0);
                // The reinjected release cannot recurse into masking.
                assert_eq!(
                    m.event(win, false, true, 0, true, true, false),
                    (false, None)
                );
                if win_first {
                    assert_eq!(hook_event(&mut m, 0x48, false), (true, None));
                }
                // Subsequent bare Win taps must still work normally.
                assert_eq!(hook_event(&mut m, win, true), (false, None));
                assert_eq!(hook_event(&mut m, win, false), (false, None));
            }
        }
    }
    #[test]
    fn hook_tracks_sided_modifiers_and_does_not_intercept_extra_modifiers() {
        let mut m = Matcher::default();
        hook_event(&mut m, 0x5b, true);
        hook_event(&mut m, 0xa3, true); // right Ctrl
        assert_eq!(m.modifiers(), WIN | CTRL);
        assert_eq!(hook_event(&mut m, 0x48, true), (false, None));
        hook_event(&mut m, 0xa3, false);
        assert_eq!(hook_event(&mut m, 0x48, true), (false, None));
        hook_event(&mut m, 0x48, false);
        hook_event(&mut m, 0x5b, false);
        assert_eq!(m.modifiers(), 0);
        hook_event(&mut m, 0x5c, true);
        hook_event(&mut m, 0xa0, true); // left Shift (physical Copilot chord)
        assert_eq!(m.modifiers(), WIN | SHIFT);
        assert_eq!(hook_event(&mut m, 0x86, true), (true, Some(Action::Toggle)));
        hook_event(&mut m, 0xa0, false);
        assert_eq!(
            hook_event(&mut m, 0x5c, false),
            (true, Some(Action::MaskWindowsRelease))
        );
        assert_eq!(hook_event(&mut m, 0x86, false), (true, None));
    }
    #[test]
    fn valid_aliases() {
        assert_eq!(
            parse(" win + shift + f23 "),
            Some(Hotkey {
                modifiers: WIN | SHIFT,
                vk: 0x86
            })
        );
        assert_eq!(
            parse("control+alt+9"),
            Some(Hotkey {
                modifiers: CTRL | ALT,
                vk: b'9' as u32
            })
        );
    }
    #[test]
    fn reject_invalid_and_duplicate() {
        for value in [
            "",
            "A",
            "Ctrl",
            "Ctrl++A",
            "Ctrl+Ctrl+A",
            "Ctrl+F25",
            "Win+A+B",
            "Win+?",
            "Alt+F0",
        ] {
            assert_eq!(parse(value), None, "{value}");
        }
    }
}

#[cfg(test)]
mod macos_tests {
    #[test]
    fn native_shortcut_aliases_and_limits() {
        for value in ["Ctrl+Alt+D", "Command+Shift+F20", "Option+9"] {
            assert!(super::valid_macos(value));
        }
        for value in [
            "D",
            "Win+H",
            "Cmd+Command+D",
            "Ctrl+F21",
            "Ctrl++D",
            "Option+Unknown",
            "Cmd+V",
        ] {
            assert!(!super::valid_macos(value));
        }
        assert!(super::parse("Command+D").is_none());
        assert!(super::parse("Win+H").is_some());
    }
}
