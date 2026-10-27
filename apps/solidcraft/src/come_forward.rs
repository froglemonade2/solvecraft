//! Bringing the window forward when a close stops at "save changes?".
//!
//! A close from the taskbar or a window switcher reaches the window while another app has the
//! focus. The prompt it opens is only seen once the window comes forward, so the window asks for
//! the focus then. A window the user is already in is left alone.
//!
//! winit's Focus asks as an application, which a window manager with focus-stealing prevention
//! (KWin's default) refuses for a window used before the switch away: it only marks the taskbar
//! entry. On X11 the request is sent again with the source EWMH gives "direct user actions"
//! (`_NET_ACTIVE_WINDOW`, source 2), which is what this is: the prompt is the rest of the user's
//! close. Native Wayland has no way for a window to raise itself, so there the attention request
//! (xdg-activation) lets the compositor raise it or mark its taskbar entry.

/// Bring the window forward, if it is in the background or minimized.
pub fn come_forward(ctx: &egui::Context, frame: &eframe::Frame) {
    let (focused, minimized) = ctx.input(|i| (i.viewport().focused, i.viewport().minimized));
    if focused == Some(true) {
        return;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    let raised = match x11_activate(frame) {
        Ok(sent) => sent,
        Err(e) => {
            log::warn!("bringing the window forward through X11: {e}");
            false
        }
    };
    #[cfg(not(all(unix, not(target_os = "macos"))))]
    let raised = {
        let _ = frame;
        false
    };
    for c in commands(minimized, raised) {
        ctx.send_viewport_cmd(c);
    }
}

/// The viewport commands for a window in the background; `raised`: the X11 request went out,
/// which also restores a minimized window.
fn commands(minimized: Option<bool>, raised: bool) -> Vec<egui::ViewportCommand> {
    if raised {
        return vec![egui::ViewportCommand::Focus];
    }
    let mut commands = Vec::new();
    if minimized == Some(true) {
        commands.push(egui::ViewportCommand::Minimized(false));
    }
    commands.push(egui::ViewportCommand::Focus);
    commands.push(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
    commands
}

/// Ask the X11 window manager to activate the window as a user action. `Ok(false)`: the window
/// isn't an X11 window (native Wayland).
#[cfg(all(unix, not(target_os = "macos")))]
fn x11_activate(frame: &eframe::Frame) -> Result<bool, String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ClientMessageEvent, ConnectionExt, EventMask};

    let handle = frame.window_handle().map_err(|e| e.to_string())?;
    let window = match handle.as_raw() {
        RawWindowHandle::Xlib(h) => u32::try_from(h.window).map_err(|e| e.to_string())?,
        RawWindowHandle::Xcb(h) => h.window.get(),
        _ => return Ok(false),
    };
    // A connection of its own: winit's isn't reachable, and a close that asks is rare.
    let (conn, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let root = conn.setup().roots.get(screen).ok_or("no X11 screen")?.root;
    let atom = conn.intern_atom(false, b"_NET_ACTIVE_WINDOW").map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?.atom;
    // Source 2: a direct user action; the time is the server's current time.
    let event = ClientMessageEvent::new(32, window, atom, [2, x11rb::CURRENT_TIME, 0, 0, 0]);
    conn.send_event(false, root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, event).map_err(|e| e.to_string())?;
    conn.flush().map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use eframe::egui::{UserAttentionType, ViewportCommand as C};

    /// Without the X11 request: focus, and the attention mark where focus is refused; a minimized
    /// window is restored first.
    #[test]
    fn a_window_closed_from_the_background_comes_forward() {
        let forward = [C::Focus, C::RequestUserAttention(UserAttentionType::Informational)];
        assert_eq!(super::commands(Some(false), false), forward);
        assert_eq!(super::commands(None, false), forward, "unknown counts as not minimized");
        let minimized = super::commands(Some(true), false);
        assert_eq!(minimized.first(), Some(&C::Minimized(false)));
        assert_eq!(minimized.get(1..), Some(&forward[..]));
        // The window manager took the X11 request: nothing left to mark.
        assert_eq!(super::commands(Some(true), true), [C::Focus]);
    }
}
