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
