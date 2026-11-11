//! Central playback shortcut mapping. Text input focus and IME are checked
//! before dispatch so space/arrows remain available to the editor.
use gpui::Keystroke;

#[derive(Debug, PartialEq, Eq)]
pub enum Shortcut {
    TogglePlayback,
    Next,
    Previous,
    Seek(i64),
    Volume(i8),
    Shuffle,
    Repeat,
    Queue,
    Search,
}

pub fn resolve(key: &Keystroke, editing: bool, composing: bool) -> Option<Shortcut> {
    let modifiers = key.modifiers;
    if composing || modifiers.alt || modifiers.platform || modifiers.function || modifiers.shift {
        return None;
    }
    if modifiers.control && key.key == "f" {
        return Some(Shortcut::Search);
    }
    if editing {
        return None;
    }
    match (modifiers.control, key.key.as_str()) {
        (false, "space") => Some(Shortcut::TogglePlayback),
        (true, "right") => Some(Shortcut::Next),
        (true, "left") => Some(Shortcut::Previous),
        (false, "right") => Some(Shortcut::Seek(5_000)),
        (false, "left") => Some(Shortcut::Seek(-5_000)),
        (true, "up") => Some(Shortcut::Volume(5)),
        (true, "down") => Some(Shortcut::Volume(-5)),
        (true, "s") => Some(Shortcut::Shuffle),
        (true, "r") => Some(Shortcut::Repeat),
        (true, "q") => Some(Shortcut::Queue),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn editing_and_ime_never_trigger_transport_shortcuts() {
        for key in ["space", "left", "right", "ctrl-right", "ctrl-s", "ctrl-q"] {
            let key = Keystroke::parse(key).unwrap();
            assert_eq!(resolve(&key, true, false), None);
            assert_eq!(resolve(&key, false, true), None);
        }
        assert_eq!(
            resolve(&Keystroke::parse("ctrl-f").unwrap(), true, false),
            Some(Shortcut::Search)
        );
    }
    #[test]
    fn arrows_distinguish_seek_from_track_navigation() {
        assert_eq!(
            resolve(&Keystroke::parse("right").unwrap(), false, false),
            Some(Shortcut::Seek(5_000))
        );
        assert_eq!(
            resolve(&Keystroke::parse("ctrl-right").unwrap(), false, false),
            Some(Shortcut::Next)
        );
        assert_eq!(
            resolve(&Keystroke::parse("shift-space").unwrap(), false, false),
            None
        );
    }
}
