//! Available power profiles and directional cycling through Omarchy's policy.
use crate::process::Runner;
use anyhow::{Context, Result, ensure};

pub struct Profiles {
    names: Vec<String>,
    active: usize,
}

impl Profiles {
    fn parse(text: &str) -> Result<Self> {
        let mut names = Vec::new();
        let mut active = None;
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            let (name, selected) = line
                .split_once('\t')
                .context("invalid power profile line")?;
            ensure!(
                matches!(name, "power-saver" | "balanced" | "performance"),
                "unknown power profile"
            );
            ensure!(
                selected == "0" || selected == "1",
                "invalid active power profile flag"
            );
            ensure!(
                !names.iter().any(|value| value == name),
                "duplicate power profile"
            );
            if selected == "1" {
                ensure!(active.is_none(), "multiple active power profiles");
                active = Some(names.len());
            }
            names.push(name.to_owned());
        }
        Ok(Self {
            names,
            active: active.context("no active power profile")?,
        })
    }

    pub async fn read(runner: &Runner) -> Result<Self> {
        Self::parse(
            &runner
                .query(&["omarchy-powerprofiles-list", "--active-state"])
                .await?,
        )
    }

    pub fn label(&self) -> String {
        self.names[self.active].replace('-', " ").to_uppercase()
    }

    fn next(&self, ticks: i16) -> &str {
        let index = (self.active as isize + ticks.signum() as isize)
            .rem_euclid(self.names.len() as isize) as usize;
        &self.names[index]
    }

    pub async fn cycle(runner: &Runner, ticks: i16) -> Result<Self> {
        let profiles = Self::read(runner).await?;
        runner
            .query(&[
                "omarchy-powerprofiles-set",
                "autodetect",
                profiles.next(ticks),
            ])
            .await?;
        // The display follows confirmation, not an assumed successful change.
        Self::read(runner).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_available_profiles_by_sign_and_wraps() {
        let profiles = Profiles::parse("power-saver\t0\nbalanced\t0\nperformance\t1\n").unwrap();
        assert_eq!(profiles.label(), "PERFORMANCE");
        assert_eq!(profiles.next(4), "power-saver");
        assert_eq!(profiles.next(-4), "balanced");
        let limited = Profiles::parse("power-saver\t1\nbalanced\t0\n").unwrap();
        assert_eq!(limited.next(-1), "balanced");
        assert!(Profiles::parse("performance\t0").is_err());
        assert!(Profiles::parse("balanced\t1\nperformance\t1").is_err());
        assert!(Profiles::parse("unexpected\t1").is_err());
    }
}
