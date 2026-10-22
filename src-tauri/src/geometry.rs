//! The main window's size, position and maximized state, kept in the app data
//! folder and restored at launch onto a screen that still exists.
//!
//! Positions use desktop units: points on macOS, where window and screen
//! positions share one space only in logical units, and physical pixels
//! elsewhere, where they share one only in physical pixels.
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use tauri::{Manager, Runtime};

const FILE: &str = "window-state.json";
/// Moves and resizes arrive continuously while dragging; save once settled.
const SETTLE: Duration = Duration::from_millis(400);
/// Logical sizes from `tauri.conf.json`.
const MIN: (f64, f64) = (760.0, 600.0);

/// `x`/`y` is the outer top-left; `width`/`height` the content size, and the
/// border the outer frame around it (title bar and window edges).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub border_width: f64,
    #[serde(default)]
    pub border_height: f64,
    #[serde(default)]
    pub maximized: bool,
}
/// A screen's work area (without menu bar, Dock or taskbar) in desktop units,
/// and the desktop units per logical pixel there.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Screen {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

/// The saved frame on the screen it overlaps most, moved and shrunk to lie
/// fully inside its work area but never below the minimum size, so the title
/// bar stays reachable. `None` when the frame is unusable or overlaps no
/// screen (a disconnected monitor): the window then opens at the default.
pub fn place(saved: &Frame, screens: &[Screen]) -> Option<Frame> {
    let sane = |v: f64| v.is_finite() && v.abs() < 1e6;
    let f = saved;
    if ![f.x, f.y, f.width, f.height, f.border_width, f.border_height]
        .into_iter()
        .all(sane)
        || f.width < 1.0
        || f.height < 1.0
        || f.border_width < 0.0
        || f.border_height < 0.0
    {
        return None;
    }
    let (right, bottom) = (
        f.x + f.width + f.border_width,
        f.y + f.height + f.border_height,
    );
    let overlap = |s: &Screen| {
        (right.min(s.x + s.width) - f.x.max(s.x)).max(0.0)
            * (bottom.min(s.y + s.height) - f.y.max(s.y)).max(0.0)
    };
    let s = screens
        .iter()
        .filter(|s| s.width > 0.0 && s.height > 0.0 && s.scale > 0.0 && overlap(s) > 0.0)
        .max_by(|a, b| overlap(a).total_cmp(&overlap(b)))?;
    let width = (f.width.min(s.width - f.border_width)).max(MIN.0 * s.scale);
    let height = (f.height.min(s.height - f.border_height)).max(MIN.1 * s.scale);
    Some(Frame {
        x: f.x.min(s.x + s.width - width - f.border_width).max(s.x),
        y: f.y.min(s.y + s.height - height - f.border_height).max(s.y),
        width,
        height,
        ..*f
    })
}
/// The state to keep after the window changed: a maximized window keeps the
/// size and position it returns to.
pub fn merge(kept: Option<Frame>, now: Frame) -> Frame {
    match kept {
        Some(kept) if now.maximized => Frame {
            maximized: true,
            ..kept
        },
        _ => now,
    }
}
pub fn read(file: &Path) -> Option<Frame> {
    serde_json::from_slice(&std::fs::read(file).ok()?).ok()
}
/// Writes through a temporary file so a crash never leaves a partial state.
pub fn write(file: &Path, frame: &Frame) {
    let Ok(bytes) = serde_json::to_vec_pretty(frame) else {
        return;
    };
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let partial = file.with_extension("json.partial");
    if std::fs::write(&partial, bytes).is_ok() {
        let _ = std::fs::rename(&partial, file);
    }
}

#[derive(Default)]
struct Inner {
    file: Option<PathBuf>,
    /// The state to save, its last written value, and the last capture.
    kept: Option<Frame>,
    written: Option<Frame>,
    latest: Option<Frame>,
    due: Option<Instant>,
}
#[derive(Default)]
pub struct Geometry(Arc<(Mutex<Inner>, Condvar)>);

impl Geometry {
    pub fn load(&self, dir: &Path) {
        let file = dir.join(FILE);
        let mut inner = self.0 .0.lock().unwrap();
        inner.kept = read(&file);
        inner.written = inner.kept;
        inner.file = Some(file);
    }
    /// Builds the main window at the saved frame, or centered at its
    /// configured size, and starts saving its changes.
    pub fn open<R: Runtime, M: Manager<R>>(
        &self,
        app: &tauri::AppHandle<R>,
        builder: tauri::WebviewWindowBuilder<'_, R, M>,
    ) -> tauri::Result<tauri::WebviewWindow<R>> {
        // The monitor list can come back empty while the main monitor is
        // known (seen when the e2e harness launches the app); the main
        // monitor then still places the saved frame, rather than nothing.
        let monitors = app
            .available_monitors()
            .ok()
            .filter(|m| !m.is_empty())
            .or_else(|| app.primary_monitor().ok().flatten().map(|m| vec![m]))
            .unwrap_or_default();
        let screens: Vec<Screen> = monitors
            .iter()
            .map(|m| {
                let (a, k) = (m.work_area(), m.scale_factor());
                let unit = if cfg!(target_os = "macos") { k } else { 1.0 };
                Screen {
                    x: a.position.x as f64 / unit,
                    y: a.position.y as f64 / unit,
                    width: a.size.width as f64 / unit,
                    height: a.size.height as f64 / unit,
                    scale: k / unit,
                }
            })
            .collect();
        let saved = self.0 .0.lock().unwrap().kept;
        let frame = saved.and_then(|f| place(&f, &screens));
        let window = match frame {
            None => builder.center().prevent_overflow().build()?,
            // Window creation takes the content's top-left in points. Creating
            // it maximized keeps that origin with the work area's size, so
            // the placed window is maximized afterwards.
            #[cfg(target_os = "macos")]
            Some(f) => {
                let window = builder
                    .position(f.x, f.y + f.border_height)
                    .inner_size(f.width, f.height)
                    .build()?;
                if f.maximized {
                    let _ = window.maximize();
                }
                window
            }
            // Creation takes logical units, which do not identify a monitor
            // when monitors differ in scale; place the hidden window instead.
            #[cfg(not(target_os = "macos"))]
            Some(f) => {
                let window = builder.visible(false).build()?;
                let _ = window.set_position(tauri::PhysicalPosition::new(
                    f.x.round() as i32,
                    f.y.round() as i32,
                ));
                let _ = window.set_size(tauri::PhysicalSize::new(
                    f.width.round() as u32,
                    f.height.round() as u32,
                ));
                if f.maximized {
                    let _ = window.maximize();
                }
                let _ = window.show();
                let _ = window.set_focus();
                window
            }
        };
        let (state, handle) = (self.0.clone(), app.clone());
        std::thread::spawn(move || saver(&state, &handle));
        Ok(window)
    }
    /// Notes a move or resize; the state is saved once it settles.
    pub fn changed<R: Runtime>(&self, app: &tauri::AppHandle<R>) {
        let now = capture(app);
        let mut inner = self.0 .0.lock().unwrap();
        if now.is_some() {
            inner.latest = now;
        }
        inner.due = Some(Instant::now() + SETTLE);
        self.0 .1.notify_one();
    }
    /// Saves at once, as the window closes or the app quits.
    pub fn flush<R: Runtime>(&self, app: &tauri::AppHandle<R>) {
        let now = capture(app);
        let mut inner = self.0 .0.lock().unwrap();
        inner.due = None;
        let now = now.or(inner.latest);
        save(&mut inner, now);
    }
}
/// Waits for changes to settle, then captures and saves the window state.
/// Window queries run on the main thread, so none is made under the lock.
fn saver<R: Runtime>(state: &(Mutex<Inner>, Condvar), app: &tauri::AppHandle<R>) {
    let (lock, wake) = state;
    loop {
        let mut inner = lock.lock().unwrap();
        loop {
            match inner.due {
                None => inner = wake.wait(inner).unwrap(),
                Some(due) if Instant::now() < due => {
                    inner = wake
                        .wait_timeout(inner, due.saturating_duration_since(Instant::now()))
                        .unwrap()
                        .0
                }
                Some(_) => break,
            }
        }
        inner.due = None;
        drop(inner);
        let now = capture(app);
        let mut inner = lock.lock().unwrap();
        // A move during the capture is saved after it settles.
        if inner.due.is_none() {
            let now = now.or(inner.latest);
            save(&mut inner, now);
        }
    }
}
fn save(inner: &mut Inner, now: Option<Frame>) {
    let Some(now) = now else { return };
    inner.kept = Some(merge(inner.kept, now));
    if inner.kept != inner.written {
        if let (Some(file), Some(frame)) = (&inner.file, &inner.kept) {
            write(file, frame);
        }
        inner.written = inner.kept;
    }
}
/// The window state now, or `None` while minimized or full screen (neither is
/// restored) or when the window is gone.
fn capture<R: Runtime>(app: &tauri::AppHandle<R>) -> Option<Frame> {
    let window = app.get_webview_window("main")?;
    if window.is_minimized().ok()? || window.is_fullscreen().ok()? {
        return None;
    }
    let maximized = window.is_maximized().ok()?;
    let (at, inner, outer) = (
        window.outer_position().ok()?,
        window.inner_size().ok()?,
        window.outer_size().ok()?,
    );
    let unit = if cfg!(target_os = "macos") {
        window.scale_factor().ok()?
    } else {
        1.0
    };
    Some(Frame {
        x: at.x as f64 / unit,
        y: at.y as f64 / unit,
        width: inner.width as f64 / unit,
        height: inner.height as f64 / unit,
        border_width: outer.width.saturating_sub(inner.width) as f64 / unit,
        border_height: outer.height.saturating_sub(inner.height) as f64 / unit,
        maximized,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn frame(x: f64, y: f64, width: f64, height: f64) -> Frame {
        Frame {
            x,
            y,
            width,
            height,
            border_width: 0.0,
            border_height: 28.0,
            maximized: false,
        }
    }
    // A 1512x982-point laptop (menu bar 33) and a 2560x1415 monitor to its left.
    const LAPTOP: Screen = Screen {
        x: 0.0,
        y: 33.0,
        width: 1512.0,
        height: 949.0,
        scale: 1.0,
    };
    const LEFT: Screen = Screen {
        x: -2560.0,
        y: -400.0,
        width: 2560.0,
        height: 1415.0,
        scale: 1.0,
    };

    #[test]
    fn a_frame_inside_a_screen_is_kept() {
        let f = frame(100.0, 80.0, 1000.0, 700.0);
        assert_eq!(place(&f, &[LAPTOP, LEFT]), Some(f));
        let left = frame(-2400.0, -300.0, 1600.0, 1000.0);
        assert_eq!(place(&left, &[LAPTOP, LEFT]), Some(left));
        let max = Frame {
            maximized: true,
            ..f
        };
        assert_eq!(place(&max, &[LAPTOP]), Some(max));
    }

    #[test]
    fn a_frame_partly_off_screen_moves_inside_the_screen_it_overlaps_most() {
        // Past the right and bottom edges.
        let f = frame(1000.0, 600.0, 1000.0, 700.0);
        let placed = place(&f, &[LAPTOP]).unwrap();
        assert_eq!((placed.x, placed.y), (512.0, 33.0 + 949.0 - 728.0));
        assert_eq!((placed.width, placed.height), (1000.0, 700.0));
        // Above the menu bar: the title bar comes back below it.
        let placed = place(&frame(10.0, 0.0, 800.0, 600.0), &[LAPTOP]).unwrap();
        assert_eq!(placed.y, 33.0);
        // Mostly on the left monitor: it stays there.
        let placed = place(&frame(-900.0, 200.0, 1000.0, 700.0), &[LAPTOP, LEFT]).unwrap();
        assert_eq!((placed.x, placed.y), (-1000.0, 200.0));
    }

    #[test]
    fn a_frame_larger_than_the_screen_shrinks_but_not_below_the_minimum() {
        let placed = place(&frame(-50.0, 10.0, 3000.0, 2000.0), &[LAPTOP]).unwrap();
        assert_eq!(placed, frame(0.0, 33.0, 1512.0, 921.0));
        let tiny = Screen {
            x: 0.0,
            y: 0.0,
            width: 700.0,
            height: 500.0,
            scale: 1.0,
        };
        let placed = place(&frame(20.0, 20.0, 900.0, 700.0), &[tiny]).unwrap();
        assert_eq!(
            placed,
            frame(0.0, 0.0, 760.0, 600.0),
            "top-left stays reachable"
        );
        // Physical pixels: the minimum scales with the screen.
        let windows = Screen {
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1032.0,
            scale: 1.5,
        };
        let placed = place(&frame(0.0, 0.0, 800.0, 600.0), &[windows]).unwrap();
        assert_eq!((placed.width, placed.height), (1140.0, 900.0));
    }

    #[test]
    fn an_unusable_frame_or_a_disconnected_screen_opens_at_the_default() {
        assert_eq!(
            place(&frame(-2000.0, 100.0, 1000.0, 700.0), &[LAPTOP]),
            None
        );
        assert_eq!(place(&frame(100.0, 100.0, 1000.0, 700.0), &[]), None);
        for bad in [
            frame(f64::NAN, 0.0, 1000.0, 700.0),
            frame(0.0, 0.0, f64::INFINITY, 700.0),
            frame(0.0, 0.0, 0.0, 700.0),
            frame(0.0, 0.0, 1000.0, -5.0),
            frame(0.0, 2e9, 1000.0, 700.0),
            Frame {
                border_height: -1.0,
                ..frame(0.0, 0.0, 1000.0, 700.0)
            },
        ] {
            assert_eq!(place(&bad, &[LAPTOP]), None, "{bad:?}");
        }
    }

    #[test]
    fn a_maximized_window_keeps_the_frame_it_returns_to() {
        let normal = frame(100.0, 80.0, 1000.0, 700.0);
        let full = Frame {
            maximized: true,
            ..frame(0.0, 33.0, 1512.0, 921.0)
        };
        let kept = merge(Some(normal), full);
        assert_eq!(
            kept,
            Frame {
                maximized: true,
                ..normal
            }
        );
        assert_eq!(
            merge(Some(kept), normal),
            normal,
            "restoring it keeps the new frame"
        );
        assert_eq!(
            merge(None, full),
            full,
            "first launch maximized: nothing else known"
        );
    }

    #[test]
    fn the_state_round_trips_and_a_broken_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("nested").join(FILE);
        assert_eq!(read(&file), None);
        let f = Frame {
            maximized: true,
            ..frame(-1200.5, 40.0, 1000.0, 700.0)
        };
        write(&file, &f);
        assert_eq!(read(&file), Some(f));
        assert!(!file.with_extension("json.partial").exists());
        let geometry = Geometry::default();
        geometry.load(file.parent().unwrap());
        assert_eq!(geometry.0 .0.lock().unwrap().kept, Some(f));
        // Older or hand-edited files may lack the optional fields.
        std::fs::write(&file, br#"{"x":1,"y":2,"width":900,"height":650}"#).unwrap();
        assert_eq!(
            read(&file),
            Some(Frame {
                border_height: 0.0,
                ..frame(1.0, 2.0, 900.0, 650.0)
            })
        );
        for broken in [&b"not json"[..], b"{\"x\":\"left\"}", b"[]", b""] {
            std::fs::write(&file, broken).unwrap();
            assert_eq!(read(&file), None);
        }
        let geometry = Geometry::default();
        geometry.load(file.parent().unwrap());
        assert_eq!(geometry.0 .0.lock().unwrap().kept, None);
        let mut inner = geometry.0 .0.lock().unwrap();
        save(&mut inner, Some(frame(5.0, 50.0, 800.0, 600.0)));
        assert_eq!(
            read(&file),
            Some(frame(5.0, 50.0, 800.0, 600.0)),
            "saving replaces it"
        );
    }
}
