//! Remembers where the window was, using the calls Windows provides for this

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub maximized: bool,
}

#[cfg(windows)]
mod native {
    use super::Placement;
    use std::ffi::c_void;

    const SW_SHOWNORMAL: u32 = 1;
    const SW_SHOWMAXIMIZED: u32 = 3;

    #[repr(C)]
    #[derive(Default)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct WindowPlacement {
        length: u32,
        flags: u32,
        show: u32,
        min_position: Point,
        max_position: Point,
        normal: Rect,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetWindowPlacement(window: *mut c_void, placement: *mut WindowPlacement) -> i32;
        fn SetWindowPlacement(window: *mut c_void, placement: *const WindowPlacement) -> i32;
    }

    pub fn read(window: u64) -> Option<Placement> {
        let mut placement = WindowPlacement {
            length: size_of::<WindowPlacement>() as u32,
            ..Default::default()
        };
        let ok = unsafe { GetWindowPlacement(window as *mut c_void, &mut placement) };
        (ok != 0).then_some(Placement {
            left: placement.normal.left,
            top: placement.normal.top,
            right: placement.normal.right,
            bottom: placement.normal.bottom,
            maximized: placement.show == SW_SHOWMAXIMIZED,
        })
    }

    pub fn apply(window: u64, saved: Placement) {
        let placement = WindowPlacement {
            length: size_of::<WindowPlacement>() as u32,
            show: if saved.maximized { SW_SHOWMAXIMIZED } else { SW_SHOWNORMAL },
            normal: Rect { left: saved.left, top: saved.top, right: saved.right, bottom: saved.bottom },
            ..Default::default()
        };
        unsafe { SetWindowPlacement(window as *mut c_void, &placement) };
    }
}

#[cfg(windows)]
pub use native::{apply, read};

#[cfg(not(windows))]
pub fn read(_window: u64) -> Option<Placement> {
    None
}

#[cfg(not(windows))]
pub fn apply(_window: u64, _saved: Placement) {}
