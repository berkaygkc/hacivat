//! Wires a `FrameRecorder` to GPUI's per-frame callback.

use crate::frames::FrameRecorder;
use crate::gpui_surface::Window;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

/// Records every frame via `Window::on_next_frame`, re-arming itself until
/// `recorder.stop()` is called.
pub fn record_frames(window: &mut Window, recorder: Rc<RefCell<FrameRecorder>>) {
    window.on_next_frame(move |window, _cx| {
        let recording = {
            let mut r = recorder.borrow_mut();
            if r.is_recording() {
                r.on_frame(Instant::now());
            }
            r.is_recording()
        };
        if recording {
            record_frames(window, recorder);
        }
    });
}
