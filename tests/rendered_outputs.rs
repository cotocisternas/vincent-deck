use vincent_deck::render::{Action, Content, Palette, Renderer, elapsed_label};

#[test]
fn opaque_dimensions_monochrome_and_visible_statuses() {
    let renderer = Renderer::new().unwrap();
    let palette = Palette::default();
    for action in Action::ALL {
        let png = renderer
            .render(action, &palette, &Content::default())
            .unwrap();
        let pixels = tiny_skia::Pixmap::decode_png(&png).unwrap();
        assert_eq!(
            (pixels.width(), pixels.height()),
            if action.panel() {
                (200, 100)
            } else {
                (144, 144)
            }
        );
        for pixel in pixels.data().as_chunks::<4>().0 {
            assert_eq!(pixel[3], 255);
            assert_eq!(pixel[0], pixel[1]);
            assert_eq!(pixel[1], pixel[2]);
        }
        let stale = renderer
            .render(
                action,
                &palette,
                &Content {
                    stale: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_ne!(png, stale, "stale {} must be visible", action.name());
        let failed = renderer
            .render(
                action,
                &palette,
                &Content {
                    failed: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_ne!(png, failed);
    }
    for seconds in [83, 3900, 5999, 7500] {
        let expected = match seconds {
            83 => "01:23",
            3900 => "65:00",
            5999 => "99:59",
            _ => "2h05m",
        };
        assert_eq!(elapsed_label(seconds), expected);
        let png = renderer
            .render(
                Action::Record,
                &palette,
                &Content {
                    active: true,
                    elapsed: elapsed_label(seconds),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_ne!(
            png,
            renderer
                .render(Action::Record, &palette, &Content::default())
                .unwrap()
        );
    }
    let normal = renderer
        .render(Action::Lock, &palette, &Content::default())
        .unwrap();
    let locked = renderer
        .render(
            Action::Lock,
            &palette,
            &Content {
                active: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_ne!(
        normal, locked,
        "confirmed lock must have visible fill/text treatment"
    );
}

#[test]
fn palette_requires_every_contract_color_and_preserves_grayscale() {
    let palette = Palette::default();
    let text = palette
        .0
        .iter()
        .map(|(k, [r, g, b])| format!("{k}\t#{r:02x}{g:02x}{b:02x}"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(Palette::parse(&text).unwrap(), palette);
    assert!(Palette::parse("bg\t#ffffff").is_err());
}

#[test]
fn audio_device_label_is_compact_and_preserves_controls_and_status() {
    let renderer = Renderer::new().unwrap();
    let palette = Palette::default();
    for action in [Action::Volume, Action::Mic] {
        let content = Content {
            percent: Some(42),
            ..Default::default()
        };
        let normal =
            tiny_skia::Pixmap::decode_png(&renderer.render(action, &palette, &content).unwrap())
                .unwrap();
        let labeled = Content {
            audio_device: "Fosi Audio ZH3".into(),
            ..content.clone()
        };
        let pixels =
            tiny_skia::Pixmap::decode_png(&renderer.render(action, &palette, &labeled).unwrap())
                .unwrap();
        let changes: Vec<_> = normal
            .data()
            .as_chunks::<4>()
            .0
            .iter()
            .zip(pixels.data().as_chunks::<4>().0)
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(index, _)| (index % 200, index / 200))
            .collect();
        assert!(!changes.is_empty());
        assert!(
            changes
                .iter()
                .all(|(x, y)| (72..186).contains(x) && (25..39).contains(y)),
            "label must not touch icon, percentage, bar, or status"
        );
        let long = Content {
            audio_device: "Very long device name ".repeat(20),
            ..labeled.clone()
        };
        let fitted =
            tiny_skia::Pixmap::decode_png(&renderer.render(action, &palette, &long).unwrap())
                .unwrap();
        for y in 25..39 {
            for x in 188..200 {
                assert_eq!(
                    pixels.pixel(x, y),
                    fitted.pixel(x, y),
                    "label must fit inside frame"
                );
            }
        }
        for (stale, muted, failed) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
        ] {
            let status = Content {
                stale,
                muted,
                failed,
                ..labeled.clone()
            };
            assert_ne!(
                renderer.render(action, &palette, &labeled).unwrap(),
                renderer.render(action, &palette, &status).unwrap()
            );
        }
    }
}

#[test]
fn stats_graphs_change_with_history_and_follow_the_palette() {
    use vincent_deck::metrics::{ACTIONS, Graph};
    let renderer = Renderer::new().unwrap();
    let palette = Palette::default();
    for action in ACTIONS {
        let mut content = Content {
            graph: Some(Graph::preview(action)),
            ..Default::default()
        };
        let original = renderer.render(action, &palette, &content).unwrap();
        content.graph.as_mut().unwrap().primary[30] = 0;
        let changed = renderer.render(action, &palette, &content).unwrap();
        assert_ne!(
            original,
            changed,
            "history must affect {} graph",
            action.name()
        );
        let mut tinted = palette.clone();
        for name in vincent_deck::render::ACCENTS {
            tinted.0.insert(name.into(), [100, 180, 220]);
        }
        assert_ne!(changed, renderer.render(action, &tinted, &content).unwrap());
        content.stale = true;
        assert_ne!(
            changed,
            renderer.render(action, &palette, &content).unwrap()
        );
    }
}

#[test]
fn cpu_power_profile_and_unavailability_are_visible() {
    let renderer = Renderer::new().unwrap();
    let palette = Palette::default();
    let mut content = Content {
        graph: Some(vincent_deck::metrics::Graph::preview(Action::Cpu)),
        power_profile: "BALANCED".into(),
        ..Default::default()
    };
    let balanced = renderer.render(Action::Cpu, &palette, &content).unwrap();
    content.power_profile = "PERFORMANCE".into();
    let performance = renderer.render(Action::Cpu, &palette, &content).unwrap();
    assert_ne!(balanced, performance);
    content.power_stale = true;
    assert_ne!(
        performance,
        renderer.render(Action::Cpu, &palette, &content).unwrap()
    );
}
