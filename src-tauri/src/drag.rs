//! Native drag-and-drop: only dragged files concern the window. WebKit also
//! reports text dragged inside the page (a selection dropped in the window),
//! without paths; such a drag shows no drop hint and opens nothing.
use std::path::PathBuf;
use tauri::DragDropEvent;

/// What a native drag event asks of the window.
#[derive(Debug, PartialEq, Eq)]
pub enum Drag {
    /// Show (true) or hide (false) the drop hint.
    Hint(bool),
    /// Hide the hint and open the dropped files as one log.
    Open(Vec<PathBuf>),
    /// Nothing to do.
    Ignore,
}

pub fn decide(event: &DragDropEvent) -> Drag {
    match event {
        DragDropEvent::Enter { paths, .. } if !paths.is_empty() => Drag::Hint(true),
        DragDropEvent::Drop { paths, .. } if !paths.is_empty() => Drag::Open(paths.clone()),
        // A drag ended or left; a hint shown for files goes away.
        DragDropEvent::Leave | DragDropEvent::Drop { .. } => Drag::Hint(false),
        _ => Drag::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::PhysicalPosition;
    const AT: PhysicalPosition<f64> = PhysicalPosition { x: 10.0, y: 20.0 };
    #[test]
    fn dragged_files_show_the_hint_and_open() {
        let paths = vec![
            PathBuf::from("synthetic-b.log"),
            PathBuf::from("synthetic-a.log"),
        ];
        let enter = DragDropEvent::Enter {
            paths: paths.clone(),
            position: AT,
        };
        assert_eq!(decide(&enter), Drag::Hint(true));
        assert_eq!(decide(&DragDropEvent::Over { position: AT }), Drag::Ignore);
        let drop = DragDropEvent::Drop {
            paths: paths.clone(),
            position: AT,
        };
        assert_eq!(decide(&drop), Drag::Open(paths));
        assert_eq!(decide(&DragDropEvent::Leave), Drag::Hint(false));
    }
    #[test]
    fn a_drag_without_files_shows_nothing_and_opens_nothing() {
        let enter = DragDropEvent::Enter {
            paths: vec![],
            position: AT,
        };
        assert_eq!(decide(&enter), Drag::Ignore);
        let drop = DragDropEvent::Drop {
            paths: vec![],
            position: AT,
        };
        assert_eq!(decide(&drop), Drag::Hint(false));
        assert_eq!(decide(&DragDropEvent::Leave), Drag::Hint(false));
    }
}
