# Control socket

A running `enzwall run` listens on `$XDG_RUNTIME_DIR/enzwall/<name>.sock`.
The default name is `default`. Use `--name` with a single filename component,
or use `--socket` to specify a path.

The socket and its directory are private to the user (0600/0700). Each request
is one UTF-8 line; each reply is one JSON line with `"ok": true|false`.
Connections may be reused, but must complete a request within three seconds of
admission or the previous completed request; partial bytes do not renew that
budget. Expired clients are reclaimed even while the wallpaper is asleep.
Custom variables retain at most 64 entries and 32 KiB across names and values.
Replacing an existing key and unsetting one reclaim the corresponding budget.

`enzwall ctl [--name N | --socket PATH] <request…>` sends a request from the
command line and exits non-zero when the reply is not ok.

| Request | Effect |
| --- | --- |
| `status` | Scene, frames rendered and `skipped`, sizes, `looping`, `transitioning`, `asleep`, `paused`, `desktop`, `rss_kib`, `partial_damage`, `smoothness`, `loop_fps`, `refresh_hz`, and pacing: `frame_ms`, `jitter_ms` and the last 240 `intervals_ms` between presented frames |
| `event unlock` / `wake` / `sleep` / `start` / `workspace` | Fire an event; `sleep` dims and then stops rendering, `unlock` and `wake` resume |
| `workspace N` | Move the parallax camera to 0-based desktop N (for compositors without an adapter) |
| `set NAME VALUE` / `unset NAME` | Custom text for `{var:NAME}` (names `[A-Za-z0-9_-]`, values ≤ 256 characters) |
| `load /abs/scene/dir` | Swap scenes without restarting; the desktop position is kept |
| `pause` / `resume` | Freeze and unfreeze looping motion (clocks still update) |
| `fps N` / `fps scene` | Override the loop frame cap, or return to the scene's own |
| `smooth balanced` / `smooth` / `auto` | Loop at the scene's cap, at the display's refresh rate, or at the refresh rate only on mains power (default; also `--smooth`) |

## Frame pacing

- **Refresh grid:** frames are timed to the output's refresh. A cap becomes a
  whole number of refreshes per frame, so 30 fps on 60 Hz is every 2nd refresh
  and 24 fps becomes 20.
- **Motion time:** motion is evaluated for the refresh the frame will appear
  on, not the moment the process woke.
- **Following the compositor:** between frames, enzwall follows the
  compositor's refresh with empty commits. With `ENZWALL_TIMER_PACING=1` it
  uses timers instead.
- **Skipping:** a frame whose largest change is under 0.3 px (or one colour
  level) is skipped, so slow motion costs less.
- **Damage:** only the areas of layers that moved are reported to the
  compositor.

## Integrating another shell

- **Start:** `enzwall run <scene> --output <name>`, which picks a
  `wlr-layer-shell` background on that output.
- **Desktops:** on Hyprland, pass `--hyprland auto` (or the instance directory);
  otherwise send `workspace N` on every desktop change.
- **Coverage:** Hyprland's native fullscreen state pauses loops on its output.
  The enzimOS compositor also reports coverage for a rendered window tagged
  `enzui-borderless-fullscreen` on the active workspace. Its main surface must
  fill the output with a known opaque buffer or a complete opaque region at
  full opacity. If the surface is clipped, rounded, or a render pass may sample
  the wallpaper for blur, loops continue. Older compositors retain the native
  fullscreen behavior.
- **Lock screens:** send `event unlock` after a successful unlock.
- **Idle:** handled by `ext-idle-notify-v1` when the compositor offers it
  (`--idle-timeout` overrides the scene; 0 never sleeps).
- **Readiness:** `--ready` prints `ready` on stdout once the first frame has
  been presented, for supervisors that swap backgrounds without a gap.
