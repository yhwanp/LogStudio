//! A native Paste menu event grants one file, once. IPC never accepts a path.
use std::{
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};

/// How long the page has to use a new grant.
const ISSUED: Duration = Duration::from_secs(5);
/// How long a held file grant waits for the user to confirm that the pasted
/// file replaces the open log.
const HELD: Duration = Duration::from_secs(600);

#[derive(Default)]
pub struct Grants(Mutex<(u64, Option<Grant>)>);
struct Grant {
    id: String,
    payload: Payload,
    expires: Instant,
    held: bool,
}
// Only the macOS Paste menu issues grants; other platforms reject every
// consume, so issuing and the payload variants are unused there.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
enum Payload {
    File(PathBuf),
    TextPrefix,
}
impl Grants {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn issue(&self, path: PathBuf) -> String {
        self.issue_payload(Payload::File(path))
    }
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn issue_prefix(&self) -> String {
        self.issue_payload(Payload::TextPrefix)
    }
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn issue_payload(&self, payload: Payload) -> String {
        let mut state = self.0.lock().unwrap();
        state.0 += 1;
        let id = format!("native-paste-{}", state.0);
        state.1 = Some(Grant {
            id: id.clone(),
            payload,
            expires: Instant::now() + ISSUED,
            held: false,
        });
        id
    }
    /// Keeps the current file grant `id` usable while the page asks whether
    /// to replace the open log. Once per grant; it stays single-use, and a
    /// newer paste or closing still supersedes it.
    pub fn hold(&self, id: &str) -> Result<(), String> {
        let mut state = self.0.lock().unwrap();
        let Some(grant) = state.1.as_mut().filter(|g| g.id == id) else {
            return Err("Paste grant is unavailable".into());
        };
        if !matches!(grant.payload, Payload::File(_)) {
            return Err("Not a file paste grant".into());
        }
        let now = Instant::now();
        if now > grant.expires {
            state.1 = None;
            return Err("Paste grant expired; paste again".into());
        }
        if grant.held {
            return Err("Paste grant is unavailable".into());
        }
        grant.held = true;
        grant.expires = now + HELD;
        Ok(())
    }
    /// Drops grant `id` unused, as when the user declines; a newer grant stays.
    pub fn discard(&self, id: &str) {
        let mut state = self.0.lock().unwrap();
        if state.1.as_ref().is_some_and(|g| g.id == id) {
            state.1 = None;
        }
    }
    pub fn consume(&self, id: &str) -> Result<PathBuf, String> {
        match self.consume_payload(id)? {
            Payload::File(path) => Ok(path),
            Payload::TextPrefix => Err("Not a file paste grant".into()),
        }
    }
    pub fn consume_prefix(&self, id: &str) -> Result<(), String> {
        match self.consume_payload(id)? {
            Payload::TextPrefix => Ok(()),
            Payload::File(_) => Err("Not a text paste grant".into()),
        }
    }
    fn consume_payload(&self, id: &str) -> Result<Payload, String> {
        let mut state = self.0.lock().unwrap();
        if !state.1.as_ref().is_some_and(|g| g.id == id) {
            return Err("Paste grant is unavailable".into());
        }
        let grant = state.1.take().unwrap();
        if Instant::now() > grant.expires {
            return Err("Paste grant expired; paste again".into());
        }
        Ok(grant.payload)
    }
    pub fn clear(&self) {
        self.0.lock().unwrap().1 = None;
    }
}

#[cfg(target_os = "macos")]
pub mod mac {
    use super::*;
    use objc2::runtime::AnyObject;
    use objc2::{sel, ClassType, MainThreadMarker};
    use objc2_app_kit::{
        NSApplication, NSPasteboard, NSPasteboardTypeFileURL, NSPasteboardTypeString,
        NSPasteboardURLReadingFileURLsOnlyKey,
    };
    use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSURL};
    use tauri::{Emitter, Manager};

    pub const PASTE: &str = "logstudio-native-paste";
    pub fn install_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
        use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
        let menu = Menu::default(app)?;
        let paste = MenuItem::with_id(app, PASTE, "Paste", true, Some("CmdOrCtrl+V"))?;
        for (position, item) in menu.items()?.iter().enumerate() {
            if item
                .as_submenu()
                .is_some_and(|s| s.text().ok().as_deref() == Some("Edit"))
            {
                let edit = Submenu::with_items(
                    app,
                    "Edit",
                    true,
                    &[
                        &PredefinedMenuItem::undo(app, None)?,
                        &PredefinedMenuItem::redo(app, None)?,
                        &PredefinedMenuItem::separator(app)?,
                        &PredefinedMenuItem::cut(app, None)?,
                        &PredefinedMenuItem::copy(app, None)?,
                        &paste,
                        &PredefinedMenuItem::select_all(app, None)?,
                    ],
                )?;
                menu.remove(item)?;
                menu.insert(&edit, position)?;
                app.set_menu(menu)?;
                return Ok(());
            }
        }
        Err(std::io::Error::other("Default Edit menu is unavailable").into())
    }
    // Read only an explicit native file URL type; a string containing a path is not a grant.
    pub fn file_on(pasteboard: &NSPasteboard) -> Result<Option<PathBuf>, String> {
        let has_file = pasteboard.types().is_some_and(|types| {
            types
                .iter()
                .any(|t| t.isEqualToString(unsafe { NSPasteboardTypeFileURL }))
        });
        if !has_file {
            return Ok(None);
        }
        let classes = NSArray::from_slice(&[NSURL::class()]);
        let yes = NSNumber::numberWithBool(true);
        let options: objc2::rc::Retained<NSDictionary<_, AnyObject>> = NSDictionary::from_slices(
            &[unsafe { NSPasteboardURLReadingFileURLsOnlyKey }],
            &[&*yes as &AnyObject],
        );
        // SAFETY: NSURL implements NSPasteboardReading. The key requires an NSNumber boolean.
        let objects = unsafe { pasteboard.readObjectsForClasses_options(&classes, Some(&options)) }
            .ok_or("Could not read the copied file; use Open File")?;
        if objects.len() != 1 {
            return Err("Paste one log file at a time".into());
        }
        let url = objects.objectAtIndex(0);
        let url = url.downcast_ref::<NSURL>().ok_or("Invalid copied file")?;
        if !url.isFileURL() {
            return Err("Paste a local file".into());
        }
        if url.host().is_some_and(|host| {
            !host.to_string().is_empty() && !host.to_string().eq_ignore_ascii_case("localhost")
        }) || url.port().is_some()
        {
            return Err("Paste a local file URL without a remote host".into());
        }
        let path = url
            .path()
            .ok_or("Copied file has no local path")?
            .to_string();
        if path.len() > 32768 || !path.starts_with('/') {
            return Err("Invalid copied file path".into());
        }
        Ok(Some(PathBuf::from(path)))
    }
    pub fn standard_paste() {
        if let Some(mtm) = MainThreadMarker::new() {
            // SAFETY: paste: is the standard NSResponder action, dispatched to the first responder.
            unsafe {
                NSApplication::sharedApplication(mtm).sendAction_to_from(sel!(paste:), None, None);
            }
        }
    }
    pub fn bom_count_on(board: &NSPasteboard) -> usize {
        let Some(data) = board.dataForType(unsafe { NSPasteboardTypeString }) else {
            return 0;
        };
        // SAFETY: Retained pasteboard NSData is only read here and never mutated.
        unsafe { data.as_bytes_unchecked() }
            .as_chunks::<3>()
            .0
            .iter()
            .take(25 * 1024 * 1024 / 3 + 1)
            .take_while(|bytes| **bytes == [0xef, 0xbb, 0xbf])
            .count()
    }
    pub fn menu_paste(app: &tauri::AppHandle) {
        if MainThreadMarker::new().is_none() {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || menu_paste(&handle));
            return;
        }
        let Some(window) = app.get_webview_window("main") else {
            return;
        };
        if !window.is_focused().unwrap_or(false) {
            return;
        }
        let board = NSPasteboard::generalPasteboard();
        match file_on(&board) {
            Ok(None) => {
                let count = bom_count_on(&board);
                if count == 0 {
                    standard_paste();
                    return;
                }
                // WebKit decodes the leading UTF-8 BOM as an encoding marker.
                // Send only its count; the actual text follows normal Paste.
                let grant = app.state::<Grants>().issue_prefix();
                if window
                    .emit(
                        "native-text-prefix",
                        serde_json::json!({"grant":grant,"count":count}),
                    )
                    .is_err()
                {
                    app.state::<Grants>().clear();
                }
            }
            Ok(Some(path)) => {
                // Only the base name, for the page to say what would open;
                // the path stays here and the page names the grant.
                let name: String = path
                    .file_name()
                    .map(|n| n.to_string_lossy().chars().take(255).collect())
                    .unwrap_or_default();
                let grant = app.state::<Grants>().issue(path);
                if window
                    .emit(
                        "native-file-paste",
                        serde_json::json!({"grant":grant,"name":name}),
                    )
                    .is_err()
                {
                    app.state::<Grants>().clear();
                }
            }
            Err(error) => {
                let _ = window.emit(
                    "source-changed",
                    serde_json::json!({"status":"error","error":error}),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sealed_grant_is_single_use_bounded_and_superseded() {
        let grants = Grants::default();
        assert!(grants.consume("arbitrary-path").is_err());
        let old = grants.issue(PathBuf::from("synthetic-old.log"));
        let current = grants.issue(PathBuf::from("synthetic-current.log"));
        assert!(grants.consume(&old).is_err());
        assert_eq!(
            grants.consume(&current).unwrap(),
            PathBuf::from("synthetic-current.log")
        );
        assert!(grants.consume(&current).is_err());
        let closed = grants.issue(PathBuf::from("synthetic-closed.log"));
        grants.clear();
        assert!(grants.consume(&closed).is_err());
    }
    #[test]
    fn held_file_grant_outlives_issue_expiry_once_and_stays_single_use() {
        let grants = Grants::default();
        let id = grants.issue(PathBuf::from("synthetic.log"));
        assert!(grants.hold("arbitrary-path").is_err());
        grants.hold(&id).unwrap();
        // Holding once more cannot extend it again.
        assert!(grants.hold(&id).is_err());
        {
            let state = grants.0.lock().unwrap();
            let grant = state.1.as_ref().unwrap();
            assert!(grant.held && grant.expires > Instant::now() + ISSUED);
        }
        assert_eq!(grants.consume(&id).unwrap(), PathBuf::from("synthetic.log"));
        assert!(grants.consume(&id).is_err());
        assert!(grants.hold(&id).is_err());
        // A held grant expires too, and a newer paste or closing supersedes it.
        let held = grants.issue(PathBuf::from("synthetic-held.log"));
        grants.hold(&held).unwrap();
        grants.0.lock().unwrap().1.as_mut().unwrap().expires =
            Instant::now() - Duration::from_secs(1);
        assert!(grants.consume(&held).unwrap_err().contains("expired"));
        let held = grants.issue(PathBuf::from("synthetic-held.log"));
        grants.hold(&held).unwrap();
        let newer = grants.issue(PathBuf::from("synthetic-newer.log"));
        assert!(grants.consume(&held).is_err());
        grants.clear();
        assert!(grants.consume(&newer).is_err());
        // Only file grants are held, and an expired grant cannot be.
        let prefix = grants.issue_prefix();
        assert!(grants.hold(&prefix).is_err());
        grants.consume_prefix(&prefix).unwrap();
        let late = grants.issue(PathBuf::from("synthetic-late.log"));
        grants.0.lock().unwrap().1.as_mut().unwrap().expires =
            Instant::now() - Duration::from_secs(1);
        assert!(grants.hold(&late).unwrap_err().contains("expired"));
        assert!(grants.consume(&late).is_err());
    }
    #[test]
    fn discarding_removes_only_that_grant() {
        let grants = Grants::default();
        let old = grants.issue(PathBuf::from("synthetic-old.log"));
        grants.hold(&old).unwrap();
        grants.discard(&old);
        assert!(grants.consume(&old).is_err());
        // A declined, superseded grant leaves the newer one usable.
        let stale = grants.issue(PathBuf::from("synthetic-stale.log"));
        let current = grants.issue_prefix();
        grants.discard(&stale);
        grants.consume_prefix(&current).unwrap();
    }
    #[test]
    fn expired_grant_cannot_open() {
        let grants = Grants::default();
        let id = grants.issue(PathBuf::from("synthetic.log"));
        grants.0.lock().unwrap().1.as_mut().unwrap().expires =
            Instant::now() - Duration::from_secs(1);
        assert!(grants.consume(&id).unwrap_err().contains("expired"));
        assert!(grants.consume(&id).is_err());
    }
    #[test]
    fn text_prefix_grant_is_typed_and_single_use() {
        let grants = Grants::default();
        let prefix = grants.issue_prefix();
        assert!(grants.consume(&prefix).is_err());
        assert!(grants.consume_prefix(&prefix).is_err());
        let file = grants.issue(PathBuf::from("synthetic.log"));
        assert!(grants.consume_prefix(&file).is_err());
        let prefix = grants.issue_prefix();
        grants.consume_prefix(&prefix).unwrap();
        assert!(grants.consume_prefix(&prefix).is_err());
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_private_pasteboard_preserves_bom_count() {
        use objc2::AnyThread;
        use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
        use objc2_foundation::{NSData, NSString};
        use std::ptr::NonNull;
        let board = NSPasteboard::pasteboardWithUniqueName();
        board.clearContents();
        // UTF-8 NSString constructors consume a leading encoding marker.
        // Create the fixture as characters, as an editor copying text would.
        let mut units: Vec<u16> = "\u{feff}raw\n".encode_utf16().collect();
        // SAFETY: The constructor copies this nonempty UTF-16 buffer.
        let text = unsafe {
            NSString::initWithCharacters_length(
                NSString::alloc(),
                NonNull::new(units.as_mut_ptr()).unwrap(),
                units.len(),
            )
        };
        assert_eq!(text.to_string(), "\u{feff}raw\n");
        assert!(board.setString_forType(&text, unsafe { NSPasteboardTypeString }));
        assert_eq!(mac::bom_count_on(&board), 1);
        board.clearContents();
        assert!(board.setData_forType(
            Some(&NSData::with_bytes("\u{feff}\u{feff}한글\n".as_bytes())),
            unsafe { NSPasteboardTypeString }
        ));
        assert_eq!(mac::bom_count_on(&board), 2);
        board.clearContents();
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_private_pasteboard_file_urls_only() {
        use objc2::runtime::ProtocolObject;
        use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString, NSPasteboardWriting};
        use objc2_foundation::{NSArray, NSString, NSURL};
        // Never read or overwrite the user's general clipboard.
        let board = NSPasteboard::pasteboardWithUniqueName();
        board.clearContents();
        assert!(
            board.setString_forType(&NSString::from_str("/tmp/synthetic.log"), unsafe {
                NSPasteboardTypeString
            })
        );
        assert_eq!(mac::file_on(&board).unwrap(), None);
        board.clearContents();
        assert!(board
            .setString_forType(&NSString::from_str("file:///tmp/synthetic.log"), unsafe {
                NSPasteboardTypeString
            }));
        assert_eq!(mac::file_on(&board).unwrap(), None);
        board.clearContents();
        let file = NSURL::fileURLWithPath(&NSString::from_str("/tmp/synthetic log.log"));
        let files =
            NSArray::from_slice(&[ProtocolObject::<dyn NSPasteboardWriting>::from_ref(&*file)]);
        assert!(board.writeObjects(&files));
        assert_eq!(
            mac::file_on(&board).unwrap(),
            Some(PathBuf::from("/tmp/synthetic log.log"))
        );
        board.clearContents();
        let other = NSURL::fileURLWithPath(&NSString::from_str("/tmp/synthetic-other.log"));
        let files = NSArray::from_slice(&[
            ProtocolObject::<dyn NSPasteboardWriting>::from_ref(&*file),
            ProtocolObject::<dyn NSPasteboardWriting>::from_ref(&*other),
        ]);
        assert!(board.writeObjects(&files));
        assert!(mac::file_on(&board).is_err());
        board.clearContents();
        assert!(board.setString_forType(
            &NSString::from_str("file://example.invalid/tmp/synthetic.log"),
            unsafe { objc2_app_kit::NSPasteboardTypeFileURL }
        ));
        assert!(mac::file_on(&board).is_err());
        board.clearContents();
        assert!(board.setString_forType(
            &NSString::from_str("https://example.invalid/synthetic.log"),
            unsafe { objc2_app_kit::NSPasteboardTypeFileURL }
        ));
        assert!(mac::file_on(&board).is_err());
        board.clearContents();
    }
}
