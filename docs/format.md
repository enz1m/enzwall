# Scene format (version 1)

A scene is a directory containing `scene.toml` and any PNG images it uses.
Unknown keys are errors, so typos never pass silently. `enzwall check <dir>`
validates a scene; `enzwall snapshot <dir> out.png --time 5` renders it.

Coordinates are fractions of the output. `x`/`y` place a layer's centre (0,0
is top-left), `w`/`h` size it, and `radius` is relative to the output height.
Colours are `#rrggbb` or `#rrggbbaa`.

## Scene keys

| Key | Default | Meaning |
| --- | --- | --- |
| `format` | required | `1` |
| `name` | required | Shown in pickers (1–64 characters) |
| `description`, `author` | `""` | Shown in pickers |
| `fps` | `30` | Frame cap for looping motion (1–60) |
| `transition_fps` | `60` | Frame cap while an event or parallax transition runs (1–120) |
| `background` | `#000000` | Clear colour under all layers |
| `render_scale` | `1.0` | Render at a fraction of the output (0.25–1); the compositor scales up |
| `idle_timeout` | `300` | Seconds without input before `sleep` fires (10–86400) |
| `sleep_dim` | `0.6` | How dark the whole scene gets while asleep (0 disables) |

`[parallax]`:

| Key | Default | Meaning |
| --- | --- | --- |
| `shift` | `0` | Fraction of the width a depth-1 layer moves per desktop |
| `duration` | `0.7` | Seconds per desktop change |
| `easing` | `in-out-cubic` | See easings below |
| `range` | `5` | Desktops the camera spans; later desktops keep the last view |

## Layers

Each `[[layer]]` draws on top of the previous one. Common keys:

- `kind`
- `id`
- `x`, `y`, `w`, `h`
- `radius`
- `rotation` (degrees)
- `scale`
- `opacity`
- `phase`
- `depth`: the parallax factor. Use 0 for layers that should stay put, such as a clock.
- `blend`: `normal`, `add` or `multiply` (ink and paper textures)

| Kind | Keys | Notes |
| --- | --- | --- |
| `solid` | `color`, `corner` | Rectangle; `corner` rounds it (fraction of output height) |
| `gradient` | `colors` (2–4), `angle`, `radial` | Dithered to avoid banding |
| `orb` | `color`, `softness` (0 hard – 1 glow), `stretch` | Circle of `radius`; `stretch` widens it into an ellipse (glints, beams) |
| `ring` | `color`, `thickness`, `softness`, `sweep`, `start`, `caps`, `bind`, `track` | Circle outline. `sweep` (0–1) draws a clockwise arc from `start` (turns, 0 = top) with `round` or `flat` caps. `bind` (`cpu`, `mem`, `battery`) drives the sweep from a live reading. `track` draws the full ring underneath |
| `wave` | `color`, `shade`, `amplitude`, `frequency`, `detail`, `softness`, `fill` | Curve across the full width at height `y`. `fill` is `down` (hills), `up`, `band` (a glowing line) or `rise` (light fading upward). `shade` fades the filled side to a second colour. `phase` scrolls it |
| `image` | `source` (relative `.png`), `fit` (`cover`/`contain`/`stretch`), `tile`, `repeat` | `tile = true` repeats horizontally, and `phase` then scrolls it in image widths. `repeat = [x, y]` tiles a small pattern (grain, dots, grids) across the box |
| `text` | `text`, `font`, `size`, `color`, `align`, `tracking`, `case`, `outline`, `outline_width`, `shadow`, `shadow_offset`, `shadow_blur` | One line; see templates. `tracking` is extra letter spacing in em. `case` is `upper` or `lower`. `outline` is a sticker border. `shadow` with zero offset and a blur is a neon glow |
| `particles` | `count`, `area`, `sizes`, `velocity`, `jitter`, `wobble`, `wobble_period`, `twinkle`, `colors`, `softness`, `stretch`, `seed`, `source`, `spin` | Up to 160 drifting glows, or a PNG sprite (`source`), wrapping inside `area = [left, top, right, bottom]`. Paths come from `seed`, so two layers with the same seed stay together (a halo and its core) |
| `grid` | `color`, `density`, `columns`, `thickness`, `fade` | A perspective floor from the box's top edge (the horizon). `phase` moves it toward the viewer, and parallax pans it |
| `graph` | `bind` (`cpu`, `mem`), `count`, `color`, `shade`, `thickness`, `glow` | The last `count` readings (one every 2 s) as a line that scrolls in, with an optional fill (`shade`) and glow |

Every layer also takes `pivot = [x, y]`, the rotation and scale origin as
fractions of its own box. For example, `[0.5, 1.0]` sways a tree from its base.

Images are at most 4096×4096. Fonts are looked up by file stem, such as
`CormorantGaramond-Light`, in this order:
1. the scene directory
2. `scenes/_shared/fonts` (the bundled fonts, with their licences)
3. `~/.local/share/fonts` and `/usr/share/fonts`

`enzwall check` fails if a named font is missing.

## Motion

`[[layer.motion]]` animates one property: `x`, `y`, `scale`, `rotation`,
`opacity` or `phase`.
- For `x`, `y`, `rotation` and `phase`, a motion adds to the base value.
- For `opacity` and `scale`, it multiplies the base value.

Looping motion (`on = "loop"`, the default):

| Key | Default | Meaning |
| --- | --- | --- |
| `wave` | `sine` | `sine`, `triangle`, `saw` (rises then snaps back) or `scroll` (endless, seamless drift; `phase` only) |
| `period` | `20` | Seconds per cycle |
| `amplitude` | `0.02` | Peak change |
| `phase` | `0` | Offset in cycles (−1–1); `x` and `y` a quarter apart trace an ellipse |
| `unit` | natural | `height` measures an `x` amplitude in output heights, so orbits stay circular |

Use `scroll` rather than `saw` for anything that should move forever. Waves
and tiled images repeat every whole unit of `phase`, so a scroll never jumps.

Reading motion (`on` = `cpu`, `mem`, `battery` or `day`) maps a live value
onto the property: `from` at 0 and `to` at 100 %. `day` runs from midnight (0)
to the next midnight (1). Readings ease in over about a second. For example, a
glow that brightens with CPU load, or a marker that turns once a day.

Event motion (`on` = `start`, `unlock`, `wake`, `sleep` or `workspace`):

| Key | Default | Meaning |
| --- | --- | --- |
| `from`, `to` | neutral | Values over the transition; neutral is 0, or 1 for opacity and scale |
| `duration` | `1` | Seconds |
| `delay` | `0` | Seconds before starting; `from` shows meanwhile |
| `easing` | `out-cubic` | See below |
| `hold` | `false` | Keep `to` afterwards instead of returning to neutral |

The most recently fired event wins per property. A typical pattern is a
`sleep` motion with `hold = true`, paired with a `wake` motion whose `from` is
the same value as the sleep motion's `to`.

Easings are:
- `linear`
- `in-quad`, `out-quad`, `in-out-quad`
- `in-cubic`, `out-cubic`, `in-out-cubic`
- `in-out-sine`, `out-expo`, `out-back`

## Text templates

`{name}` or `{name:argument}`; `{{` and `}}` are literal braces.

| Variable | Value | Refresh |
| --- | --- | --- |
| `time` | `strftime` format, default `%H:%M` | per minute, or per second with `%S` |
| `date` | `strftime` format, default `%A, %-d %B` | per minute |
| `cpu`, `mem` | Percent used | every 2 s |
| `mem_used` | e.g. `5.2 GiB` | every 2 s |
| `load` | 1-minute load average | every 2 s |
| `uptime` | e.g. `3h 12m` | per minute |
| `battery` | Percent, or `–` | every 30 s |
| `host`, `user` | Host and user name | once |
| `var:name` | Set with `enzwall ctl set name value` | when set |

The renderer wakes only for the fastest refresh any visible template or bound
ring needs.

## Composition tips

- **Depth sandwich:** put the clock between landscape layers, with a smaller
  `depth` than the layers in front of it. The nearer layers then cross the
  numerals and slide past them on desktop changes.
- **Parallax travel:** keep `depth × shift × (range − 1)` under about 0.15 for
  anything that must stay readable.
- **Scene changes:** switching scenes crossfades for 0.8 s.
