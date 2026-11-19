use enzwall_scene::Trigger;
use rustix::net::{AddressFamily, SocketAddrUnix, SocketFlags, SocketType, connect, socket_with};
use std::io;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(crate) enum Command {
    Status,
    Event(Trigger),
    Workspace(u32),
    Set(String, String),
    Unset(String),
    Load(PathBuf),
    Pause,
    Resume,
    Fps(Option<u32>),
    Smooth(crate::pacing::Smoothness),
}

const MAX_LINE: usize = 4096;

fn connect_control(path: &Path) -> io::Result<UnixStream> {
    let socket = socket_with(
        AddressFamily::UNIX,
        SocketType::STREAM,
        SocketFlags::NONBLOCK | SocketFlags::CLOEXEC,
        None,
    )?;
    connect(&socket, &SocketAddrUnix::new(path)?)?;
    Ok(UnixStream::from(socket))
}

fn var_name(name: &str) -> Result<String, String> {
    if !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Ok(name.into())
    } else {
        Err("variable names are 1–32 of [A-Za-z0-9_-]".into())
    }
}

pub(crate) fn parse(line: &str) -> Result<Command, String> {
    let line = line.trim();
    let (verb, rest) = line
        .split_once(' ')
        .map_or((line, ""), |(v, r)| (v, r.trim()));
    Ok(match verb {
        "status" => Command::Status,
        "event" => Command::Event(
            Trigger::parse(rest)
                .filter(|t| *t != Trigger::Loop)
                .ok_or("event must be start, unlock, wake, sleep or workspace")?,
        ),
        "workspace" => Command::Workspace(
            rest.parse::<u32>()
                .ok()
                .filter(|n| *n < 10_000)
                .ok_or("workspace needs a 0-based index")?,
        ),
        "set" => {
            let (name, value) = rest.split_once(' ').ok_or("set needs a name and a value")?;
            if value.chars().count() > 256 {
                return Err("values are limited to 256 characters".into());
            }
            Command::Set(var_name(name)?, value.to_owned())
        }
        "unset" => Command::Unset(var_name(rest)?),
        "load" => {
            let path = PathBuf::from(rest);
            if !path.is_absolute() {
                return Err("load needs an absolute scene directory".into());
            }
            Command::Load(path)
        }
        "pause" => Command::Pause,
        "resume" => Command::Resume,
        "smooth" => Command::Smooth(
            crate::pacing::Smoothness::parse(rest)
                .ok_or("smooth takes balanced, smooth or auto")?,
        ),
        "fps" => Command::Fps(match rest {
            "scene" => None,
            n => Some(
                n.parse::<u32>()
                    .ok()
                    .filter(|n| (1..=60).contains(n))
                    .ok_or("fps needs 1–60 or \"scene\"")?,
            ),
        }),
        _ => return Err(format!("unknown command {verb:?}")),
    })
}

pub(crate) fn default_socket(name: &str) -> Result<PathBuf, String> {
    if Path::new(name).file_name() != Some(name.as_ref()) || name.chars().any(char::is_control) {
        return Err("--name must be a single socket name; use --socket for a path".into());
    }
    let runtime =
        std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set; pass --socket")?;
    Ok(Path::new(&runtime)
        .join("enzwall")
        .join(format!("{name}.sock")))
}

struct Client {
    id: u64,
    stream: UnixStream,
    buffer: Vec<u8>,
    deadline: Instant,
}

/// Local, owner-only control socket. Each request is one line; each reply is
/// one JSON line.
pub(crate) struct Server {
    path: PathBuf,
    listener: UnixListener,
    identity: (u64, u64),
    clients: Vec<Client>,
    next_id: u64,
}

impl Server {
    pub(crate) fn bind(path: &Path) -> Result<Self, String> {
        let uid = rustix::process::geteuid().as_raw();
        let dir = path.parent().ok_or("socket needs a parent directory")?;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
        let parent = std::fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
        if !parent.is_dir() || parent.uid() != uid || parent.mode() & 0o022 != 0 {
            return Err(
                "socket directory must be owned by this account and not writable by others".into(),
            );
        }
        match std::fs::symlink_metadata(path) {
            Ok(meta) => {
                if !meta.file_type().is_socket() || meta.uid() != uid {
                    return Err("refusing to replace a foreign socket or unrelated file".into());
                }
                match connect_control(path) {
                    Ok(_) => {
                        return Err(format!(
                            "{} is already in use by another enzwall",
                            path.display()
                        ));
                    }
                    Err(e) if e.kind() == ErrorKind::ConnectionRefused => {
                        let current = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
                        if (current.dev(), current.ino()) != (meta.dev(), meta.ino()) {
                            return Err("control socket changed during startup".into());
                        }
                        std::fs::remove_file(path).map_err(|e| e.to_string())?;
                    }
                    Err(e) => return Err(e.to_string()),
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        let listener = UnixListener::bind(path).map_err(|e| format!("{}: {e}", path.display()))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        Ok(Self {
            path: path.to_path_buf(),
            listener,
            identity: (meta.dev(), meta.ino()),
            clients: Vec::new(),
            next_id: 0,
        })
    }

    pub(crate) fn poll_fds(&self, fds: &mut Vec<libc::pollfd>) {
        fds.push(libc::pollfd {
            fd: self.listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        });
        for client in &self.clients {
            fds.push(libc::pollfd {
                fd: client.stream.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            });
        }
    }

    pub(crate) fn timeout(&self, timeout: Option<Duration>) -> Option<Duration> {
        let idle = self
            .clients
            .iter()
            .map(|client| client.deadline.saturating_duration_since(Instant::now()))
            .min();
        match (timeout, idle) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Returns requests keyed by a client id that stays valid while other
    /// clients disconnect.
    pub(crate) fn receive(&mut self) -> Vec<(u64, Result<Command, String>)> {
        let now = Instant::now();
        self.clients.retain(|client| client.deadline > now);
        for _ in 0..16 {
            let Ok((stream, _)) = self.listener.accept() else {
                break;
            };
            let owned = rustix::net::sockopt::socket_peercred(&stream)
                .is_ok_and(|peer| peer.uid == rustix::process::geteuid());
            if owned && self.clients.len() < 16 && stream.set_nonblocking(true).is_ok() {
                self.next_id += 1;
                self.clients.push(Client {
                    id: self.next_id,
                    stream,
                    buffer: Vec::new(),
                    deadline: now + Duration::from_secs(3),
                });
            }
        }
        let mut commands = Vec::new();
        let mut closed = Vec::new();
        for (index, client) in self.clients.iter_mut().enumerate() {
            let mut chunk = [0u8; 1024];
            loop {
                match client.stream.read(&mut chunk) {
                    Ok(0) => {
                        closed.push(index);
                        break;
                    }
                    Ok(n) => client.buffer.extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(_) => {
                        closed.push(index);
                        break;
                    }
                }
                if client.buffer.len() > MAX_LINE {
                    closed.push(index);
                    break;
                }
            }
            while let Some(end) = client.buffer.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = client.buffer.drain(..=end).collect();
                client.deadline = now + Duration::from_secs(3);
                commands.push((
                    index,
                    client.id,
                    std::str::from_utf8(&line)
                        .map_err(|_| "requests must be UTF-8".to_owned())
                        .and_then(parse),
                ));
            }
        }
        for &(index, _, _) in &commands {
            closed.retain(|&c| c != index);
        }
        closed.sort_unstable();
        closed.dedup();
        for index in closed.into_iter().rev() {
            self.clients.remove(index);
        }
        commands
            .into_iter()
            .map(|(_, id, command)| (id, command))
            .collect()
    }

    pub(crate) fn reply(&mut self, id: u64, value: &serde_json::Value) {
        if let Some(client) = self.clients.iter_mut().find(|c| c.id == id) {
            let mut line = value.to_string();
            line.push('\n');
            let _ = client.stream.set_nonblocking(false);
            let _ = client
                .stream
                .set_write_timeout(Some(Duration::from_millis(200)));
            let _ = client.stream.write_all(line.as_bytes());
            let _ = client.stream.set_nonblocking(true);
        }
    }

    pub(crate) fn remove(&self) {
        if std::fs::symlink_metadata(&self.path).is_ok_and(|meta| {
            meta.file_type().is_socket() && (meta.dev(), meta.ino()) == self.identity
        }) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Sends one request and returns the JSON reply line.
pub(crate) fn request(path: &Path, line: &str) -> Result<String, String> {
    let line = line.trim();
    if line.len() >= MAX_LINE {
        return Err("request too long".into());
    }
    let mut stream = connect_control(path)
        .map_err(|e| format!("{}: {e} (is enzwall running?)", path.display()))?;
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    stream
        .write_all(format!("{line}\n").as_bytes())
        .map_err(|e| e.to_string())?;
    let mut reply = Vec::new();
    let mut byte = [0u8; 1];
    while stream.read(&mut byte).map_err(|e| e.to_string())? == 1 && byte[0] != b'\n' {
        reply.push(byte[0]);
        if reply.len() > 65536 {
            return Err("reply too long".into());
        }
    }
    String::from_utf8(reply).map_err(|_| "reply is not UTF-8".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_socket_preserves_peers_and_unrelated_files() {
        let directory = tempfile::tempdir().expect("private socket fixture");
        let path = directory.path().join("control.sock");
        std::fs::write(&path, b"keep").expect("private socket fixture");
        assert!(Server::bind(&path).is_err());
        assert_eq!(
            std::fs::read(&path).expect("private socket fixture"),
            b"keep"
        );
        std::fs::remove_file(&path).expect("private socket fixture");
        let server = Server::bind(&path).expect("private socket fixture");
        assert!(Server::bind(&path).is_err());
        drop(server);
        let mut replacement = Server::bind(&path).expect("private socket fixture");
        let mut client = UnixStream::connect(&path).expect("private socket fixture");
        client
            .write_all(b"status\n")
            .expect("private socket fixture");
        let requests = replacement.receive();
        assert!(matches!(requests.as_slice(), [(_, Ok(Command::Status))]));
        replacement.reply(requests[0].0, &serde_json::json!({"ok":true}));
        let mut bytes = [0; 12];
        client
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("private socket fixture");
        client
            .read_exact(&mut bytes)
            .expect("private socket fixture");
        assert_eq!(&bytes, b"{\"ok\":true}\n");
        let held: Vec<_> = (0..16)
            .map(|_| UnixStream::connect(&path).unwrap())
            .collect();
        replacement.receive();
        assert_eq!(replacement.clients.len(), 16);
        for client in &mut replacement.clients {
            client.deadline = Instant::now() - Duration::from_secs(1);
        }
        assert_eq!(replacement.timeout(None), Some(Duration::ZERO));
        let mut after_expiry = UnixStream::connect(&path).unwrap();
        after_expiry.write_all(b"status\n").unwrap();
        assert!(matches!(
            replacement.receive().as_slice(),
            [(_, Ok(Command::Status))]
        ));
        drop(held);
        std::fs::remove_file(&path).expect("private socket fixture");
        std::fs::write(&path, b"new owner").expect("private socket fixture");
        replacement.remove();
        assert_eq!(
            std::fs::read(&path).expect("private socket fixture"),
            b"new owner"
        );
        std::fs::remove_file(&path).expect("private socket fixture");
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o777))
            .expect("private socket fixture");
        assert!(Server::bind(&path).is_err());
    }

    #[test]
    fn requests_parse_strictly() {
        assert!(matches!(parse("status\n"), Ok(Command::Status)));
        assert!(matches!(
            parse("event unlock"),
            Ok(Command::Event(Trigger::Unlock))
        ));
        assert!(parse("event loop").is_err());
        assert!(matches!(parse("workspace 3"), Ok(Command::Workspace(3))));
        assert!(parse("workspace -1").is_err());
        assert!(
            matches!(parse("set now_playing Blue in Green"), Ok(Command::Set(n, v)) if n == "now_playing" && v == "Blue in Green")
        );
        assert!(parse("set ../x 1").is_err());
        assert!(parse("load relative/scene").is_err());
        assert!(matches!(parse("fps scene"), Ok(Command::Fps(None))));
        assert!(matches!(parse("fps 12"), Ok(Command::Fps(Some(12)))));
        assert!(parse("fps 0").is_err());
        assert!(matches!(
            parse("smooth auto"),
            Ok(Command::Smooth(crate::pacing::Smoothness::Auto))
        ));
        assert!(parse("smooth fast").is_err());
        assert!(parse("reboot").is_err());
        for name in [
            "",
            ".",
            "..",
            "/absolute",
            "../parent",
            "subdir/name",
            "name\n",
        ] {
            assert!(default_socket(name).unwrap_err().starts_with("--name"));
        }
    }
}
