//! Runs, inspects and captures animated Wayland wallpaper packages.
#![allow(
    clippy::print_stdout,
    reason = "This CLI writes replies, captures and benchmark reports to stdout; diagnostics use stderr."
)]
mod egl;
mod gl;
mod hyprland;
mod ipc;
mod pacing;
mod run;
mod snapshot;
mod stage;
mod text;
mod vars;

use enzwall_scene::Trigger;
use snapshot::Cue;
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
enzwall — animated, parallax and live-information wallpapers for Wayland

USAGE:
  enzwall run <scene-dir> [--output NAME] [--name NAME | --socket PATH] [--namespace NS]
              [--hyprland auto|DIR] [--idle-timeout SECONDS] [--smooth balanced|smooth|auto] [--ready]
              [--hide-clock] [--hide-date]
  enzwall ctl [--name NAME | --socket PATH] <status | event NAME | workspace N | set VAR VALUE
              | unset VAR | load DIR | pause | resume | fps N|scene | smooth balanced|smooth|auto>
  enzwall list <scenes-dir>
  enzwall check <scene-dir>
  enzwall snapshot <scene-dir> <out.png> [--size WxH] [--time S] [--frames N --interval S]
                   [--event NAME@S]... [--workspace N@S]... [--clock UNIX]
  enzwall stream <scene-dir> [--size WxH] [--fps 1..30] [--hide-clock] [--hide-date] [--static]
  enzwall bench <scene-dir> [--size WxH] [--frames N]
";

fn size(value: &str) -> Result<(u32, u32), String> {
    let (w, h) = value.split_once('x').ok_or("size must be WxH")?;
    let (w, h) = (
        w.parse::<u32>().map_err(|e| e.to_string())?,
        h.parse::<u32>().map_err(|e| e.to_string())?,
    );
    if !(16..=8192).contains(&w) || !(16..=8192).contains(&h) {
        return Err("size must be 16–8192 on each side".into());
    }
    Ok((w, h))
}

fn seconds(value: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|time| time.is_finite())
        .ok_or_else(|| format!("expected finite seconds, got {value:?}"))
}

fn at(value: &str) -> Result<(&str, f64), String> {
    let (name, time) = value.split_once('@').ok_or("expected NAME@SECONDS")?;
    Ok((name, seconds(time)?))
}

fn frame_count(value: &str) -> Result<u32, String> {
    value
        .parse::<u32>()
        .ok()
        .filter(|count| *count != 0)
        .ok_or_else(|| "--frames must be a positive integer".into())
}

struct Args(std::vec::IntoIter<String>);

impl Args {
    fn positional(&mut self, what: &str) -> Result<String, String> {
        self.0
            .next()
            .ok_or_else(|| format!("missing {what}\n\n{USAGE}"))
    }
    fn value(&mut self, flag: &str) -> Result<String, String> {
        self.0.next().ok_or_else(|| format!("{flag} needs a value"))
    }
}

fn run() -> Result<(), String> {
    let mut args = Args(std::env::args().skip(1).collect::<Vec<_>>().into_iter());
    let command = args.0.next().unwrap_or_default();
    match command.as_str() {
        "run" => {
            let scene = std::fs::canonicalize(args.positional("scene directory")?)
                .map_err(|e| format!("scene: {e}"))?;
            let (mut output, mut name, mut socket, mut namespace, mut hypr, mut ready, mut idle) = (
                None,
                "default".to_owned(),
                None,
                "enzwall".to_owned(),
                None,
                false,
                None,
            );
            let mut smooth = pacing::Smoothness::Auto;
            let (mut hide_clock, mut hide_date) = (false, false);
            while let Some(flag) = args.0.next() {
                match flag.as_str() {
                    "--output" => output = Some(args.value(&flag)?),
                    "--name" => name = args.value(&flag)?,
                    "--socket" => socket = Some(PathBuf::from(args.value(&flag)?)),
                    "--namespace" => namespace = args.value(&flag)?,
                    "--hyprland" => hypr = Some(args.value(&flag)?),
                    "--ready" => ready = true,
                    "--hide-clock" => hide_clock = true,
                    "--hide-date" => hide_date = true,
                    "--idle-timeout" => {
                        idle = Some(
                            args.value(&flag)?
                                .parse()
                                .map_err(|_| "invalid --idle-timeout")?,
                        );
                    }
                    "--smooth" => {
                        smooth = pacing::Smoothness::parse(&args.value(&flag)?)
                            .ok_or("--smooth takes balanced, smooth or auto")?;
                    }
                    other => return Err(format!("unknown option {other}\n\n{USAGE}")),
                }
            }
            let hyprland = match hypr.as_deref() {
                None => None,
                Some("auto") => match (
                    std::env::var_os("XDG_RUNTIME_DIR"),
                    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE"),
                ) {
                    (Some(runtime), Some(instance)) => {
                        Some(PathBuf::from(runtime).join("hypr").join(instance))
                    }
                    _ => None,
                },
                Some(dir) => Some(PathBuf::from(dir)),
            };
            let socket = match socket {
                Some(path) => path,
                None => ipc::default_socket(&name)?,
            };
            run::run(run::Options {
                scene,
                output,
                namespace,
                socket,
                hyprland,
                ready,
                idle_timeout: idle,
                smooth,
                hide_clock,
                hide_date,
            })
        }
        "ctl" => {
            let mut socket = None;
            let mut words = Vec::new();
            while let Some(word) = args.0.next() {
                match word.as_str() {
                    "--name" if words.is_empty() => {
                        socket = Some(ipc::default_socket(&args.value(&word)?)?);
                    }
                    "--socket" if words.is_empty() => {
                        socket = Some(PathBuf::from(args.value(&word)?));
                    }
                    _ => words.push(word),
                }
            }
            if words.is_empty() {
                return Err(format!("ctl needs a command\n\n{USAGE}"));
            }
            let line = words.join(" ");
            ipc::parse(&line)?;
            let socket = match socket {
                Some(path) => path,
                None => ipc::default_socket("default")?,
            };
            let reply = ipc::request(&socket, &line)?;
            println!("{reply}");
            let ok = serde_json::from_str::<serde_json::Value>(&reply)
                .ok()
                .is_some_and(|v| v["ok"] == true);
            if ok {
                Ok(())
            } else {
                Err("request failed".into())
            }
        }
        "list" => {
            let dir = PathBuf::from(args.positional("scenes directory")?);
            let mut scenes = Vec::new();
            let mut entries: Vec<_> = std::fs::read_dir(&dir)
                .map_err(|e| format!("{}: {e}", dir.display()))?
                .flatten()
                .collect();
            entries.sort_by_key(std::fs::DirEntry::file_name);
            for entry in entries {
                if let Ok(scene) = stage::read_scene(&entry.path()) {
                    scenes.push(serde_json::json!({
                        "id": entry.file_name().to_string_lossy(),
                        "name": scene.name,
                        "description": scene.description,
                        "animated": scene.has_loops(),
                        "parallax": scene.parallax.shift > 0.0,
                        "preview": entry.path().join("preview.png").is_file().then(|| entry.path().join("preview.png")),
                    }));
                }
            }
            println!("{}", serde_json::Value::Array(scenes));
            Ok(())
        }
        "check" => {
            let dir = PathBuf::from(args.positional("scene directory")?);
            let scene = stage::read_scene(&dir)?;
            for source in scene.images() {
                stage::decode_png(&dir.join(source))?;
            }
            let mut fonts = text::Fonts::new();
            for layer in &scene.layers {
                if let enzwall_scene::Kind::Text(t) = &layer.kind
                    && let Some(name) = &t.font
                    && !fonts.exists(&dir, name)
                {
                    return Err(format!(
                        "font {name:?} not found in the scene, scenes/_shared/fonts or the system"
                    ));
                }
            }
            println!(
                "{}: {} layers, {} fps loops{}, parallax {}",
                scene.name,
                scene.layers.len(),
                scene.fps,
                if scene.has_loops() {
                    ""
                } else {
                    " (none, static)"
                },
                scene.parallax.shift
            );
            Ok(())
        }
        "stream" => {
            let dir = PathBuf::from(args.positional("scene directory")?);
            let (mut dims, mut fps, mut hide_clock, mut hide_date, mut still) =
                ((960, 540), 15, false, false, false);
            while let Some(flag) = args.0.next() {
                match flag.as_str() {
                    "--size" => dims = size(&args.value(&flag)?)?,
                    "--fps" => {
                        fps = args
                            .value(&flag)?
                            .parse::<u32>()
                            .map_err(|_| "invalid fps")?
                            .clamp(1, 30);
                    }
                    "--hide-clock" => hide_clock = true,
                    "--hide-date" => hide_date = true,
                    "--static" => still = true,
                    _ => return Err("unknown stream option".into()),
                }
            }
            if dims.0 > 1920 || dims.1 > 1080 {
                return Err("stream size is limited to 1920×1080".into());
            }
            snapshot::stream(&dir, dims, fps, hide_clock, hide_date, still)
        }
        "snapshot" => {
            let dir = PathBuf::from(args.positional("scene directory")?);
            let out = PathBuf::from(args.positional("output PNG")?);
            let (mut dims, mut time, mut frames, mut interval, mut cues, mut clock) =
                ((1920, 1080), 0.0, 1, 0.1, Vec::new(), None);
            while let Some(flag) = args.0.next() {
                match flag.as_str() {
                    "--size" => dims = size(&args.value(&flag)?)?,
                    "--time" => time = seconds(&args.value(&flag)?)?,
                    "--frames" => {
                        frames = frame_count(&args.value(&flag)?)?;
                    }
                    "--interval" => {
                        interval = seconds(&args.value(&flag)?)?;
                        if interval < 0.0 {
                            return Err("--interval must be non-negative".into());
                        }
                    }
                    "--clock" => {
                        clock = Some(args.value(&flag)?.parse().map_err(|_| "invalid --clock")?);
                    }
                    "--event" => {
                        let value = args.value(&flag)?;
                        let (name, t) = at(&value)?;
                        cues.push(Cue::Event(
                            Trigger::parse(name).ok_or_else(|| format!("unknown event {name}"))?,
                            t,
                        ));
                    }
                    "--workspace" => {
                        let value = args.value(&flag)?;
                        let (index, t) = at(&value)?;
                        cues.push(Cue::Workspace(
                            index.parse().map_err(|_| "invalid workspace")?,
                            t,
                        ));
                    }
                    other => return Err(format!("unknown option {other}\n\n{USAGE}")),
                }
            }
            if !(time + f64::from(frames - 1) * interval).is_finite() {
                return Err("snapshot end time is out of range".into());
            }
            snapshot::snapshot(&dir, &out, dims, time, frames, interval, &cues, clock)
        }
        "bench" => {
            let dir = PathBuf::from(args.positional("scene directory")?);
            let (mut dims, mut frames) = ((1920, 1080), 600);
            while let Some(flag) = args.0.next() {
                match flag.as_str() {
                    "--size" => dims = size(&args.value(&flag)?)?,
                    "--frames" => {
                        frames = frame_count(&args.value(&flag)?)?;
                    }
                    other => return Err(format!("unknown option {other}")),
                }
            }
            let result = snapshot::bench(&dir, dims, frames)?;
            let gpu = match (result.gpu_ms, result.gpu_worst_ms) {
                (Some(mean), Some(worst)) => format!("GPU {mean:.3} ms (worst {worst:.3})"),
                _ => "GPU time unavailable (no timer queries)".into(),
            };
            println!(
                "{frames} frames at {}x{}: CPU {:.3} ms/frame, {gpu}",
                dims.0, dims.1, result.cpu_ms
            );
            Ok(())
        }
        "" | "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command {other}\n\n{USAGE}")),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("enzwall: {error}");
            ExitCode::FAILURE
        }
    }
}
