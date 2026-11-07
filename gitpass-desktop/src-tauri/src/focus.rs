//! App in focus, through the macOS Accessibility API (read-only): the focused window's document
//! (`AXDocument`, which Terminal and iTerm set to the shell's folder and VS Code / Cursor to the open
//! file) and title. Used only when the user already granted the permission; never asks for it.

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusedWindow {
    pub pid: i32,
    pub document: Option<String>,
    pub title: Option<String>,
}

#[cfg(target_os = "macos")]
mod mac {
    use std::ffi::c_void;

    use core_foundation::base::{CFType, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringRef};
    use core_foundation::url::CFURL;

    use super::FocusedWindow;

    type AXUIElementRef = *const c_void;
    const AX_SUCCESS: i32 = 0;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> u8;
        fn AXUIElementCreateSystemWide() -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(element: AXUIElementRef, attribute: CFStringRef, value: *mut CFTypeRef) -> i32;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, seconds: f32) -> i32;
        fn AXUIElementGetPid(element: AXUIElementRef, pid: *mut i32) -> i32;
    }

    pub fn trusted() -> bool {
        // SAFETY: no arguments; only reads whether this process has the permission.
        unsafe { AXIsProcessTrusted() != 0 }
    }

    /// Owned `AXUIElementRef` / attribute value (released on drop).
    fn attribute(element: &CFType, name: &'static str) -> Option<CFType> {
        let attr = CFString::from_static_string(name);
        let mut value: CFTypeRef = std::ptr::null();
        // SAFETY: `element` is a live AXUIElement; on success `value` is a +1 reference we own.
        let err = unsafe { AXUIElementCopyAttributeValue(element.as_CFTypeRef(), attr.as_concrete_TypeRef(), &mut value) };
        if err != AX_SUCCESS || value.is_null() {
            return None;
        }
        // SAFETY: Copy rule: the caller owns `value`.
        Some(unsafe { CFType::wrap_under_create_rule(value) })
    }

    fn text(v: &CFType) -> Option<String> {
        if let Some(s) = v.downcast::<CFString>() {
            return Some(s.to_string()).filter(|s| !s.is_empty());
        }
        v.downcast::<CFURL>().and_then(|u| u.to_path()).map(|p| p.display().to_string())
    }

    pub fn focused_window() -> Option<FocusedWindow> {
        if !trusted() {
            return None;
        }
        // SAFETY: returns a +1 AXUIElement that `CFType` releases.
        let system = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateSystemWide() as CFTypeRef) };
        // A busy app must not stall the watcher (the default timeout is several seconds).
        // SAFETY: `system` is a live AXUIElement.
        unsafe { AXUIElementSetMessagingTimeout(system.as_CFTypeRef(), 0.25) };
        let app = attribute(&system, "AXFocusedApplication")?;
        let mut pid = 0;
        // SAFETY: `app` is a live AXUIElement; `pid` is a valid out pointer.
        if unsafe { AXUIElementGetPid(app.as_CFTypeRef(), &mut pid) } != AX_SUCCESS {
            return None;
        }
        let window = attribute(&app, "AXFocusedWindow").or_else(|| attribute(&app, "AXMainWindow"));
        let (document, title) = match &window {
            Some(w) => (
                attribute(w, "AXDocument").and_then(|v| text(&v)),
                attribute(w, "AXTitle").and_then(|v| text(&v)),
            ),
            None => (None, None),
        };
        Some(FocusedWindow { pid, document, title })
    }
}

/// The user allowed GitPass in System Settings › Privacy & Security › Accessibility.
pub fn accessibility_granted() -> bool {
    #[cfg(target_os = "macos")]
    {
        mac::trusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Focused window of the frontmost app; `None` without the permission or when nothing is focused.
pub fn focused_window() -> Option<FocusedWindow> {
    #[cfg(target_os = "macos")]
    {
        mac::focused_window()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn querying_focus_is_safe_with_or_without_the_permission() {
        let granted = super::accessibility_granted();
        for _ in 0..3 {
            let w = super::focused_window();
            if !granted {
                assert!(w.is_none());
            }
        }
    }
}
