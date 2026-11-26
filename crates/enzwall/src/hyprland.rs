use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(crate) enum HyprEvent {
    /// 0-based desktop index on this wallpaper's output.
    Workspace(u32),
    /// Something that can cover the wallpaper changed; ask [`Hyprland::covered`].
    Coverage,
}

/// Follows Hyprland's event socket (`.socket2.sock`) for desktop changes on
/// one output. Other compositors drive enzwall through `ctl workspace`.
pub(crate) struct Hyprland {
    dir: PathBuf,
    events: UnixStream,
    buffer: Vec<u8>,
    focused: Option<String>,
    output: Option<String>,
}

fn index(id: &str) -> Option<u32> {
    id.trim()
        .parse::<i64>()
        .ok()
        .filter(|id| *id >= 1)
        .map(|id| (id - 1).min(9_999) as u32)
}

impl Hyprland {
    /// `dir` is `$XDG_RUNTIME_DIR/hypr/<instance>`.
    pub(crate) fn connect(dir: &Path) -> Result<Self, String> {
        let events = UnixStream::connect(dir.join(".socket2.sock"))
            .map_err(|e| format!("Hyprland events in {}: {e}", dir.display()))?;
        events.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(Self {
            dir: dir.to_path_buf(),
            events,
            buffer: Vec::new(),
            focused: None,
            output: None,
        })
    }

    pub(crate) fn fd(&self) -> &UnixStream {
        &self.events
    }

    pub(crate) fn active_workspace(&mut self, output: Option<&str>) -> Option<u32> {
        self.output = output.map(str::to_owned);
        let mut stream = UnixStream::connect(self.dir.join(".socket.sock")).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        stream.write_all(b"j/monitors").ok()?;
        let mut reply = String::new();
        stream.take(1 << 20).read_to_string(&mut reply).ok()?;
        let monitors: serde_json::Value = serde_json::from_str(&reply).ok()?;
        let monitors = monitors.as_array()?;
        let monitor = monitors
            .iter()
            .find(|m| output.is_none_or(|o| m["name"] == o))
            .or_else(|| monitors.first())?;
        if monitor["focused"] == true {
            self.focused = monitor["name"].as_str().map(str::to_owned);
        }
        index(&monitor["activeWorkspace"]["id"].to_string())
    }

    /// Whether the active desktop is covered by native fullscreen or by an
    /// opaque borderless window confirmed by the compositor's renderer.
    pub(crate) fn covered(&self) -> Option<bool> {
        let monitors = self.query(b"j/monitors")?;
        let clients = self.query(b"j/clients")?;
        covers(&monitors, &clients, self.output.as_deref())
    }

    fn query(&self, request: &[u8]) -> Option<serde_json::Value> {
        let mut stream = UnixStream::connect(self.dir.join(".socket.sock")).ok()?;
        stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
        stream.write_all(request).ok()?;
        let mut reply = String::new();
        stream.take(4 << 20).read_to_string(&mut reply).ok()?;
        serde_json::from_str(&reply).ok()
    }

    pub(crate) fn receive(&mut self) -> Vec<HyprEvent> {
        let mut chunk = [0u8; 4096];
        loop {
            match self.events.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => self.buffer.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
            if self.buffer.len() > 1 << 20 {
                self.buffer.clear();
            }
        }
        let mut out = Vec::new();
        let mut coverage = false;
        while let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let Some((event, data)) = line.trim_end().split_once(">>") else {
                continue;
            };
            match event {
                "focusedmon" => self.focused = data.split(',').next().map(str::to_owned),
                "workspacev2" => {
                    coverage = true;
                    let ours = self.output.is_none()
                        || self.focused.is_none()
                        || self.focused == self.output;
                    if let (true, Some(i)) = (ours, data.split(',').next().and_then(index)) {
                        out.push(HyprEvent::Workspace(i));
                    }
                }
                "enzui-wallpaper-coverage" if self.output.as_deref().is_none_or(|o| o == data) => {
                    coverage = true;
                }
                "fullscreen" | "openwindow" | "closewindow" | "movewindowv2" | "monitoradded"
                | "monitorremoved" | "minimized" => coverage = true,
                _ => {}
            }
        }
        if coverage {
            out.push(HyprEvent::Coverage);
        }
        out
    }
}

/// [`Hyprland::covered`] from the `j/monitors` and `j/clients` replies.
fn covers(
    monitors: &serde_json::Value,
    clients: &serde_json::Value,
    output: Option<&str>,
) -> Option<bool> {
    let monitor = monitors
        .as_array()?
        .iter()
        .find(|m| output.is_none_or(|output| m["name"] == output))?;
    if monitor["opaqueBorderlessCover"] == true {
        return Some(true);
    }
    Some(clients.as_array()?.iter().any(|client| {
        client["fullscreen"] == 2
            && client["mapped"] == true
            && client["hidden"] == false
            && client["monitor"] == monitor["id"]
            && client["workspace"]["id"] == monitor["activeWorkspace"]["id"]
    }))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::os::unix::net::UnixStream;

    #[test]
    fn opaque_borderless_coverage_is_per_output_and_requires_compositor_confirmation() {
        let monitors = serde_json::json!([
            {"name": "A", "opaqueBorderlessCover": true},
            {"name": "B", "opaqueBorderlessCover": false},
            {"name": "C"},
        ]);
        let clients = serde_json::json!([{
            "fullscreen": 0, "mapped": true, "hidden": false,
            "tags": ["enzui-borderless-fullscreen"],
        }]);
        assert_eq!(super::covers(&monitors, &clients, Some("A")), Some(true));
        assert_eq!(super::covers(&monitors, &clients, Some("B")), Some(false));
        assert_eq!(super::covers(&monitors, &clients, Some("C")), Some(false));
    }

    #[test]
    fn borderless_coverage_events_refresh_only_the_named_output() {
        let (events, mut sender) = UnixStream::pair().expect("event socket");
        events
            .set_nonblocking(true)
            .expect("nonblocking event socket");
        let mut hyprland = super::Hyprland {
            dir: Default::default(),
            events,
            buffer: Vec::new(),
            focused: None,
            output: Some("A".into()),
        };
        sender
            .write_all(b"enzui-wallpaper-coverage>>B\n")
            .expect("event for B");
        assert!(hyprland.receive().is_empty());
        sender
            .write_all(b"enzui-wallpaper-coverage>>A\n")
            .expect("event for A");
        assert!(matches!(
            hyprland.receive().as_slice(),
            [super::HyprEvent::Coverage]
        ));
    }

    #[test]
    fn workspace_ids_map_to_zero_based_desktops() {
        assert_eq!(super::index("1"), Some(0));
        assert_eq!(super::index(" 4"), Some(3));
        assert_eq!(super::index("-98"), None);
        assert_eq!(super::index("special"), None);
    }

    /// Sabotage: accept maximized windows (`fullscreen` 1). The maximized
    /// case then reads as covered.
    #[test]
    fn only_a_fullscreen_window_on_the_shown_desktop_covers_the_output() {
        let monitors = serde_json::json!([
            {"id": 0, "name": "A", "activeWorkspace": {"id": 1}},
            {"id": 1, "name": "B", "activeWorkspace": {"id": 4}},
        ]);
        let client = |fullscreen: u8, monitor: u8, workspace: u8| {
            serde_json::json!({"fullscreen": fullscreen, "mapped": true, "hidden": false,
                "monitor": monitor, "workspace": {"id": workspace}})
        };
        let covers =
            |clients: serde_json::Value, output| super::covers(&monitors, &clients, output);
        assert_eq!(
            covers(serde_json::json!([client(2, 0, 1)]), Some("A")),
            Some(true)
        );
        assert_eq!(
            covers(serde_json::json!([client(1, 0, 1)]), Some("A")),
            Some(false)
        );
        assert_eq!(
            covers(serde_json::json!([client(2, 0, 2)]), Some("A")),
            Some(false)
        );
        assert_eq!(
            covers(serde_json::json!([client(2, 0, 1)]), Some("B")),
            Some(false)
        );
        assert_eq!(
            covers(serde_json::json!([client(2, 1, 4)]), Some("B")),
            Some(true)
        );
        assert_eq!(covers(serde_json::json!([]), Some("C")), None);
    }
}
