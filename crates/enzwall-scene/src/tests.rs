use super::*;

const SCENE: &str = r##"
format = 1
name = "Test"
fps = 24
background = "#101820"
[parallax]
shift = 0.1
duration = 1.0
easing = "linear"

[[layer]]
kind = "gradient"
colors = ["#000000", "#ffffff"]

[[layer]]
kind = "orb"
id = "glow"
x = 0.3
depth = 2.0
color = "#ff000080"
[[layer.motion]]
property = "y"
amplitude = 0.1
period = 4
[[layer.motion]]
property = "opacity"
on = "sleep"
from = 1
to = 0.2
duration = 2
hold = true
[[layer.motion]]
property = "opacity"
on = "wake"
from = 0.2
to = 1
duration = 1

[[layer]]
kind = "text"
text = "It is {time:%H:%M} — {{cpu}} {cpu}%"
"##;

fn evaluate_at(scene: &Scene, runtime: &Runtime, t: f64) -> Evaluated {
    evaluate(scene, runtime, t, 1.6)
}

fn scene() -> Scene {
    Scene::parse(SCENE).expect("valid scene fixture")
}

#[test]
fn rejects_unknown_fields_bad_values_and_escaping_images() {
    for (edit, needle) in [
        (
            SCENE.replace("fps = 24", "fps = 24\nspeed = 2"),
            "unknown field",
        ),
        (SCENE.replace("format = 1", "format = 2"), "format"),
        (SCENE.replace("#ff000080", "red"), "colour"),
        (SCENE.replace("period = 4", "period = 0"), "period"),
        (
            SCENE.replace("kind = \"orb\"", "kind = \"video\""),
            "unknown layer kind",
        ),
        (SCENE.replace("{cpu}%", "{gpu}%"), "unknown variable"),
        (
            SCENE.replace("kind = \"orb\"", "kind = \"image\"\nsource = \"../x.png\""),
            "relative .png",
        ),
    ] {
        let error = Scene::parse(&edit).unwrap_err();
        assert!(error.contains(needle), "{error}");
    }
}

#[test]
fn loops_are_additive_and_pausing_freezes_them() {
    let scene = scene();
    let mut runtime = Runtime::new(0.0);
    let at = |runtime: &Runtime, t| evaluate(&scene, runtime, t, 1.6).layers[1].props.y;
    assert!((at(&runtime, 1.0) - 0.6).abs() < 1e-5);
    assert!((at(&runtime, 3.0) - 0.4).abs() < 1e-5);
    runtime.pause_loops(1.0);
    assert!((at(&runtime, 3.0) - 0.6).abs() < 1e-5);
    assert!(!evaluate_at(&scene, &runtime, 3.0).looping);
    runtime.resume_loops(5.0);
    assert!((at(&runtime, 5.0) - 0.6).abs() < 1e-5);
    assert!((at(&runtime, 7.0) - 0.4).abs() < 1e-5);
}

#[test]
fn event_transitions_hold_and_the_latest_trigger_wins() {
    let scene = scene();
    let mut runtime = Runtime::new(0.0);
    let opacity = |runtime: &Runtime, t| evaluate(&scene, runtime, t, 1.6).layers[1].props.opacity;
    assert_eq!(opacity(&runtime, 1.0), 1.0);
    runtime.fire(Trigger::Sleep, 10.0);
    assert!((opacity(&runtime, 11.0) - 0.3).abs() < 1e-5);
    assert!(evaluate_at(&scene, &runtime, 11.0).transitioning);
    assert!((opacity(&runtime, 20.0) - 0.2).abs() < 1e-5);
    assert!(!evaluate_at(&scene, &runtime, 20.0).transitioning);
    runtime.fire(Trigger::Wake, 30.0);
    assert!((opacity(&runtime, 30.0) - 0.2).abs() < 1e-5);
    assert_eq!(opacity(&runtime, 40.0), 1.0);
}

#[test]
fn parallax_shift_follows_depth_and_eases_between_desktops() {
    let scene = scene();
    let mut runtime = Runtime::new(0.0);
    runtime.workspace(&scene, 2, 10.0);
    let mid = evaluate_at(&scene, &runtime, 10.5);
    assert!(mid.transitioning);
    assert!((mid.layers[1].shift + 0.2).abs() < 1e-5);
    assert_eq!(mid.layers[0].shift, 0.0);
    let end = evaluate_at(&scene, &runtime, 12.0);
    assert!(!end.transitioning);
    assert!((end.layers[1].shift + 0.4).abs() < 1e-5);
    runtime.workspace(&scene, 0, 12.0);
    assert!((evaluate_at(&scene, &runtime, 12.5).layers[1].shift + 0.2).abs() < 1e-5);
    runtime.jump(3, 20.0);
    assert!(!evaluate_at(&scene, &runtime, 20.0).transitioning);
    assert_eq!(runtime.desktop(), 3);
}

#[test]
fn templates_render_escape_braces_and_report_cadence() {
    let template = Template::parse("It is {time:%H:%M} — {{cpu}} {cpu}%").unwrap();
    let text = template.render(|name, arg| {
        format!(
            "<{name}{}>",
            arg.map(|a| format!(":{a}")).unwrap_or_default()
        )
    });
    assert_eq!(text, "It is <time:%H:%M> — {cpu} <cpu>%");
    assert_eq!(template.cadence(), Cadence::TwoSeconds);
    assert_eq!(
        Template::parse("{time:%H:%M:%S}").unwrap().cadence(),
        Cadence::Second
    );
    assert_eq!(
        Template::parse("{date}").unwrap().cadence(),
        Cadence::Minute
    );
    assert_eq!(
        Template::parse("hello {host}").unwrap().cadence(),
        Cadence::Never
    );
    assert!(Template::parse("{var:now_playing}").is_ok());
    assert!(Template::parse("{var:../x}").is_err());
    assert!(Template::parse("{time").is_err());
}

#[test]
fn easings_hit_their_endpoints() {
    for name in [
        "linear",
        "in-quad",
        "out-quad",
        "in-out-quad",
        "in-cubic",
        "out-cubic",
        "in-out-cubic",
        "in-out-sine",
        "out-expo",
        "out-back",
    ] {
        let easing = Easing::parse(name).unwrap();
        assert!(easing.apply(0.0).abs() < 1e-5, "{name}");
        assert!((easing.apply(1.0) - 1.0).abs() < 1e-5, "{name}");
    }
}

#[test]
fn height_units_keep_orbits_circular() {
    let scene = Scene::parse(&SCENE.replace(
        "property = \"y\"\namplitude = 0.1",
        "property = \"x\"\nunit = \"height\"\namplitude = 0.1",
    ))
    .unwrap();
    let runtime = Runtime::new(0.0);
    assert!((evaluate(&scene, &runtime, 1.0, 2.0).layers[1].props.x - 0.35).abs() < 1e-5);
    assert!(
        Scene::parse(&SCENE.replace("amplitude = 0.1", "unit = \"height\"\namplitude = 0.1"))
            .is_err()
    );
}

#[test]
fn sleep_veil_fades_in_holds_and_lifts_on_wake() {
    let scene = scene();
    let mut runtime = Runtime::new(0.0);
    assert_eq!(evaluate_at(&scene, &runtime, 1.0).dim, 0.0);
    runtime.fire(Trigger::Sleep, 10.0);
    assert!(evaluate_at(&scene, &runtime, 10.5).transitioning);
    assert!((evaluate_at(&scene, &runtime, 20.0).dim - 0.6).abs() < 1e-5);
    runtime.fire(Trigger::Unlock, 30.0);
    assert!(evaluate_at(&scene, &runtime, 30.2).dim > 0.0);
    assert_eq!(evaluate_at(&scene, &runtime, 32.0).dim, 0.0);
}

#[test]
fn parallax_range_stops_the_camera_at_the_last_desktop_view() {
    let scene =
        Scene::parse(&SCENE.replace("easing = \"linear\"", "easing = \"linear\"\nrange = 3"))
            .unwrap();
    let mut runtime = Runtime::new(0.0);
    runtime.workspace(&scene, 7, 0.0);
    let end = evaluate_at(&scene, &runtime, 5.0);
    assert!((end.layers[1].shift + 0.4).abs() < 1e-5);
    runtime.workspace(&scene, 9, 6.0);
    assert!(!evaluate_at(&scene, &runtime, 6.1).transitioning);
    assert_eq!(runtime.desktop(), 9);
}

#[test]
fn scroll_drift_is_continuous_across_cycle_boundaries() {
    let scene = Scene::parse(&SCENE.replace(
        "property = \"y\"\namplitude = 0.1\nperiod = 4",
        "property = \"phase\"\nwave = \"scroll\"\namplitude = 1\nperiod = 4",
    ))
    .unwrap();
    let runtime = Runtime::new(0.0);
    let phase = |t: f64| evaluate_at(&scene, &runtime, t).layers[1].props.phase;
    let step = |a: f32, b: f32| {
        let d = (b - a).rem_euclid(1.0);
        d.min(1.0 - d)
    };
    let mut t = 0.0;
    while t < 400_000.0 {
        assert!(step(phase(t), phase(t + 1.0 / 30.0)) < 0.01, "jump at {t}");
        t += 3.97;
    }
    assert!(
        Scene::parse(&SCENE.replace(
            "amplitude = 0.1\nperiod = 4",
            "wave = \"scroll\"\namplitude = 0.1\nperiod = 4"
        ))
        .is_err()
    );
}

#[test]
fn rings_bind_gauges_and_orbs_stretch() {
    let scene = Scene::parse(&SCENE.replace(
        "kind = \"orb\"",
        "kind = \"ring\"\nbind = \"cpu\"\nsweep = 0.5\ntrack = \"#ffffff22\"",
    ))
    .unwrap();
    assert!(scene.has_gauges());
    assert!(
        matches!(scene.layers[1].kind, Kind::Ring { bind: Some(Source::Cpu), sweep, .. } if sweep == 0.5)
    );
    assert!(
        Scene::parse(&SCENE.replace("kind = \"orb\"", "kind = \"ring\"\nbind = \"gpu\"")).is_err()
    );
    let glint =
        Scene::parse(&SCENE.replace("kind = \"orb\"", "kind = \"orb\"\nstretch = 8")).unwrap();
    assert!(matches!(glint.layers[1].kind, Kind::Orb { stretch, .. } if stretch == 8.0));
}

#[test]
fn reading_motions_map_live_values_and_ease_between_samples() {
    let scene = Scene::parse(&SCENE.replace(
        "property = \"y\"\namplitude = 0.1\nperiod = 4",
        "property = \"y\"\non = \"mem\"\nfrom = 0.2\nto = -0.2",
    ))
    .unwrap();
    assert_eq!(scene.readings(), vec![Source::Mem]);
    let mut runtime = Runtime::new(0.0);
    assert!((evaluate_at(&scene, &runtime, 1.0).layers[1].props.y - 0.5).abs() < 1e-5);
    runtime.set_reading(Source::Mem, 0.5, 1.0);
    assert!((evaluate_at(&scene, &runtime, 1.0).layers[1].props.y - 0.5).abs() < 1e-5);
    runtime.set_reading(Source::Mem, 1.0, 2.0);
    let mid = evaluate_at(&scene, &runtime, 2.6);
    assert!(!mid.transitioning && mid.layers[1].props.y < 0.5 && mid.layers[1].props.y > 0.3);
    assert!((evaluate_at(&scene, &runtime, 5.0).layers[1].props.y - 0.3).abs() < 1e-5);
}

#[test]
fn new_layer_kinds_validate_their_options() {
    let with = |layer: &str| Scene::parse(&format!("format = 1\nname = \"t\"\n[[layer]]\n{layer}"));
    assert!(
        matches!(with("kind = \"particles\"\ncount = 30\nsizes = [0.001, 0.004]\nvelocity = [0.0, 0.02]\ncolors = [\"#ffffff\", \"#ffeecc\"]").unwrap().layers[0].kind, Kind::Particles(ref p) if p.count == 30 && p.colors.len() == 2)
    );
    assert!(with("kind = \"particles\"\narea = [0.5, 0.0, 0.2, 1.0]").is_err());
    assert!(
        matches!(with("kind = \"grid\"\ncolor = \"#ff00ff\"\ndensity = 8").unwrap().layers[0].kind, Kind::Grid { density, .. } if density == 8.0)
    );
    assert!(matches!(
        with("kind = \"graph\"\nbind = \"cpu\"\ncount = 40")
            .unwrap()
            .layers[0]
            .kind,
        Kind::Graph { samples: 40, .. }
    ));
    assert!(with("kind = \"graph\"\nbind = \"day\"").is_err());
    let text = with("kind = \"text\"\ntext = \"{time}\"\ntracking = 0.2\ncase = \"upper\"\noutline = \"#ffffff\"\nshadow = \"#00000080\"\nshadow_offset = [0.02, 0.05]").unwrap();
    assert!(
        matches!(text.layers[0].kind, Kind::Text(ref t) if t.case == Case::Upper && t.outline.is_some() && t.shadow.is_some_and(|s| s.y == 0.05))
    );
    assert_eq!(
        with("kind = \"solid\"\npivot = [0.5, 1.0]\ncorner = 0.02")
            .unwrap()
            .layers[0]
            .pivot,
        (0.5, 1.0)
    );
    assert!(with("kind = \"image\"\nsource = \"a.png\"\nrepeat = [4, 200]").is_ok());
}

#[test]
fn wallpaper_roles_are_explicit_or_inferred_and_filter_independently() {
    let scene = Scene::parse(
        r##"format=1
name="Roles"
[[layer]]
kind="text"
text="{time:%H:%M}"
[[layer]]
kind="text"
text="{date:%A}"
[[layer]]
kind="solid"
role="clock"
[[layer]]
kind="solid"
"##,
    )
    .unwrap();
    assert_eq!(
        scene.layers.iter().map(|l| l.role).collect::<Vec<_>>(),
        vec![
            LayerRole::Clock,
            LayerRole::Date,
            LayerRole::Clock,
            LayerRole::Decoration
        ]
    );
    assert_eq!(
        scene
            .layers
            .iter()
            .filter(|l| !l.role.hidden(true, false))
            .count(),
        2
    );
    assert_eq!(
        scene
            .layers
            .iter()
            .filter(|l| !l.role.hidden(false, true))
            .count(),
        3
    );
    assert_eq!(
        scene
            .layers
            .iter()
            .filter(|l| !l.role.hidden(true, true))
            .count(),
        1
    );
    assert!(
        Scene::parse("format=1\nname='Bad'\n[[layer]]\nkind='solid'\nrole='executable'").is_err()
    );
    assert!(
        Scene::parse("format=1\nname='Bad'\n[[layer]]\nkind='text'\ntext='Hi'\nfont='/etc/shadow'")
            .is_err()
    );
}
