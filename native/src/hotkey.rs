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

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Action {
    Toggle,
    Stop,
}
/// Allocation-free key-repeat/suppressed-up tracking for the low-level hook.
pub struct Matcher {
    suppressed: [bool; 256],
    pressed: [bool; 256],
    escape: bool,
}
impl Default for Matcher {
    fn default() -> Self {
        Self {
            suppressed: [false; 256],
            pressed: [false; 256],
            escape: false,
        }
    }
}
impl Matcher {
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
