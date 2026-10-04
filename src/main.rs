use anyhow::Result;
use vincent_deck::render::{Action, Content, Palette, Renderer};

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--render-samples") {
        let output = args.get(2).expect("output directory required");
        std::fs::create_dir_all(output)?;
        let renderer = Renderer::new()?;
        let palette = std::process::Command::new("omarchy-theme-color")
            .arg("--all")
            .output()?;
        let palette = Palette::parse(std::str::from_utf8(&palette.stdout)?)?;
        for action in Action::ALL {
            let content = Content {
                percent: Some(42),
                audio_device: match action {
                    Action::Volume => "Fosi Audio ZH3",
                    Action::Mic => "AT2020USB+",
                    _ => "",
                }
                .into(),
                workspace: Some(3),
                window: "firefox: GitHub".into(),
                theme: "Rose Pine".into(),
                position: "7/25".into(),
                power_profile: "BALANCED".into(),
                graph: action
                    .stats()
                    .then(|| vincent_deck::metrics::Graph::preview(action)),
                ..Default::default()
            };
            let png = renderer.render(action, &palette, &content)?;
            std::fs::write(format!("{output}/{}.png", action.name()), png)?;
        }
        return Ok(());
    }
    for flag in ["-port", "-pluginUUID", "-registerEvent", "-info"] {
        anyhow::ensure!(
            args.windows(2)
                .any(|pair| pair[0].eq_ignore_ascii_case(flag)),
            "missing {flag}"
        );
    }
    let renderer = Renderer::new()?;
    let app = vincent_deck::app::App::new(renderer);
    vincent_deck::actions::register(app.clone()).await;
    app.start();
    let mut delay = 1;
    loop {
        app.reconnect().await;
        if let Err(error) = openaction::run(args.clone()).await {
            eprintln!("host connection: {error}");
        }
        tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        delay = (delay * 2).min(10);
    }
}
