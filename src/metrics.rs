//! Read-only Linux telemetry. Counter deltas use actual elapsed time and histories
//! are bounded; disappearing/reset devices never become throughput spikes.
use crate::render::Action;
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    time::Instant,
};

pub const HISTORY: usize = 60;
pub const ACTIONS: [Action; 4] = [Action::Cpu, Action::Memory, Action::Disk, Action::Network];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Graph {
    pub primary: VecDeque<u64>,
    pub secondary: VecDeque<u64>,
    pub value: String,
    pub detail: String,
    pub scale: u64,
    pub paired: bool,
}

impl Graph {
    /// Representative static preview for the manifest icons and sample renderer.
    pub fn preview(action: Action) -> Self {
        let mut graph = Self::new(matches!(action, Action::Disk | Action::Network));
        for i in 0..HISTORY {
            let value = 200 + ((i * 37) % 550) as u64;
            graph.push(value, value / 2, "".into(), "".into());
        }
        if graph.paired {
            graph.value = "R 42.0KiB/s".into();
            graph.detail = "W 18.0KiB/s".into();
            if action == Action::Network {
                graph.value = "D 42.0KiB/s".into();
                graph.detail = "U 18.0KiB/s".into();
            }
        } else {
            graph.value = "42.0%".into();
            graph.detail = if action == Action::Memory {
                "6.7GiB / 16.0GiB"
            } else {
                "ALL CORES  60s"
            }
            .into();
        }
        graph
    }

    fn new(paired: bool) -> Self {
        Self {
            primary: VecDeque::new(),
            secondary: VecDeque::new(),
            value: "--".into(),
            detail: "WARMING UP".into(),
            scale: 1000,
            paired,
        }
    }

    fn push(&mut self, primary: u64, secondary: u64, value: String, detail: String) {
        if self.primary.len() == HISTORY {
            self.primary.pop_front();
            self.secondary.pop_front();
        }
        self.primary.push_back(primary);
        self.secondary.push_back(secondary);
        self.value = value;
        self.detail = detail;
        self.scale = if self.paired {
            self.primary
                .iter()
                .chain(&self.secondary)
                .copied()
                .max()
                .unwrap_or(0)
                .max(1024)
        } else {
            1000
        };
    }
}

type Counters = BTreeMap<String, (u64, u64)>;

#[derive(Default)]
struct RateBaseline(Option<(Instant, Counters)>);

impl RateBaseline {
    fn update(&mut self, counters: Counters, now: Instant) -> Option<(u64, u64)> {
        let (before, previous) = self.0.replace((now, counters.clone()))?;
        let seconds = now.duration_since(before).as_secs_f64();
        if seconds <= 0.0 {
            return None;
        }
        let mut delta = (0_u64, 0_u64);
        for (name, (a, b)) in counters {
            if let Some((old_a, old_b)) = previous.get(&name) {
                delta.0 = delta.0.saturating_add(a.saturating_sub(*old_a));
                delta.1 = delta.1.saturating_add(b.saturating_sub(*old_b));
            }
        }
        Some((
            (delta.0 as f64 / seconds) as u64,
            (delta.1 as f64 / seconds) as u64,
        ))
    }
}

pub struct Sampler {
    proc: PathBuf,
    sys: PathBuf,
    cpu: Option<(u64, u64)>,
    disk: RateBaseline,
    network: RateBaseline,
    graphs: [Graph; 4],
}

impl Default for Sampler {
    fn default() -> Self {
        Self::new("/proc", "/sys")
    }
}

impl Sampler {
    pub fn new(proc: impl Into<PathBuf>, sys: impl Into<PathBuf>) -> Self {
        Self {
            proc: proc.into(),
            sys: sys.into(),
            cpu: None,
            disk: RateBaseline::default(),
            network: RateBaseline::default(),
            graphs: [
                Graph::new(false),
                Graph::new(false),
                Graph::new(true),
                Graph::new(true),
            ],
        }
    }

    pub async fn sample(&mut self) -> Vec<(Action, Result<Graph>)> {
        let cpu = self.sample_cpu().await;
        let memory = self.sample_memory().await;
        let disk = self.sample_rates(false).await;
        let network = self.sample_rates(true).await;
        ACTIONS
            .into_iter()
            .zip([cpu, memory, disk, network])
            .collect()
    }

    async fn sample_cpu(&mut self) -> Result<Graph> {
        let result = async {
            let text = tokio::fs::read_to_string(self.proc.join("stat")).await?;
            cpu_counters(&text)
        }
        .await;
        let (total, idle) = match result {
            Ok(value) => value,
            Err(error) => {
                self.cpu = None;
                return Err(error);
            }
        };
        let Some((previous_total, previous_idle)) = self.cpu.replace((total, idle)) else {
            bail!("CPU counter baseline warming up");
        };
        let delta = total
            .checked_sub(previous_total)
            .context("CPU counter reset")?;
        let idle = idle
            .checked_sub(previous_idle)
            .context("CPU idle counter reset")?;
        ensure!(delta > 0 && idle <= delta, "invalid CPU counter interval");
        let usage = ((delta - idle) as f64 / delta as f64 * 1000.0).round() as u64;
        self.graphs[0].push(usage, 0, percent(usage), "ALL CORES  60s".into());
        Ok(self.graphs[0].clone())
    }

    async fn sample_memory(&mut self) -> Result<Graph> {
        let text = tokio::fs::read_to_string(self.proc.join("meminfo")).await?;
        let (used, total) = memory_bytes(&text)?;
        let usage = (used as f64 / total as f64 * 1000.0).round() as u64;
        self.graphs[1].push(
            usage,
            0,
            percent(usage),
            format!("{} / {}", bytes(used), bytes(total)),
        );
        Ok(self.graphs[1].clone())
    }

    async fn sample_rates(&mut self, network: bool) -> Result<Graph> {
        let result = self.read_counters(network).await;
        let baseline = if network {
            &mut self.network
        } else {
            &mut self.disk
        };
        let counters = match result {
            Ok(counters) => counters,
            Err(error) => {
                baseline.0 = None;
                return Err(error);
            }
        };
        let count = counters.len();
        let Some((a, b)) = baseline.update(counters, Instant::now()) else {
            bail!("throughput counter baseline warming up");
        };
        let graph = &mut self.graphs[if network { 3 } else { 2 }];
        graph.push(
            a,
            b,
            format!("{} {}", if network { "D" } else { "R" }, rate(a)),
            format!(
                "{} {}  {count} {}",
                if network { "U" } else { "W" },
                rate(b),
                if network { "NIC" } else { "DEV" }
            ),
        );
        Ok(graph.clone())
    }

    async fn read_counters(&self, network: bool) -> Result<Counters> {
        let text = tokio::fs::read_to_string(self.proc.join(if network {
            "net/dev"
        } else {
            "diskstats"
        }))
        .await?;
        let all = if network {
            network_counters(&text)?
        } else {
            disk_counters(&text)?
        };
        let mut physical = Counters::new();
        for (name, counters) in all {
            let path = if network {
                self.sys.join("class/net").join(&name).join("device")
            } else {
                self.sys.join("block").join(&name).join("device")
            };
            if tokio::fs::try_exists(path).await? {
                physical.insert(name, counters);
            }
        }
        // Empty is valid: a machine with no physical NIC/disk has zero throughput.
        Ok(physical)
    }
}

fn cpu_counters(text: &str) -> Result<(u64, u64)> {
    let mut words = text
        .lines()
        .next()
        .context("missing CPU counters")?
        .split_whitespace();
    ensure!(words.next() == Some("cpu"), "missing aggregate CPU line");
    // guest and guest_nice are already included in user/nice; do not double count.
    let values: Vec<u64> = words
        .take(8)
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    ensure!(values.len() >= 4, "incomplete CPU counters");
    let total = values
        .iter()
        .try_fold(0_u64, |sum, n| sum.checked_add(*n))
        .context("CPU overflow")?;
    let idle = values[3]
        .checked_add(values.get(4).copied().unwrap_or(0))
        .context("CPU idle overflow")?;
    Ok((total, idle))
}

fn memory_bytes(text: &str) -> Result<(u64, u64)> {
    let field = |name: &str| -> Result<u64> {
        let line = text
            .lines()
            .find(|line| line.starts_with(name))
            .context("missing memory field")?;
        let mut words = line.split_whitespace().skip(1);
        let value: u64 = words.next().context("missing memory value")?.parse()?;
        ensure!(words.next() == Some("kB"), "invalid memory unit");
        value.checked_mul(1024).context("memory overflow")
    };
    let total = field("MemTotal:")?;
    let available = field("MemAvailable:")?;
    ensure!(total > 0 && available <= total, "invalid memory totals");
    Ok((total - available, total))
}

fn disk_counters(text: &str) -> Result<Counters> {
    let mut counters = Counters::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let words: Vec<_> = line.split_whitespace().collect();
        ensure!(words.len() >= 14, "incomplete disk counters");
        let sectors = |i: usize| -> Result<u64> {
            words[i]
                .parse::<u64>()?
                .checked_mul(512)
                .context("disk counter overflow")
        };
        counters.insert(words[2].into(), (sectors(5)?, sectors(9)?));
    }
    Ok(counters)
}

fn network_counters(text: &str) -> Result<Counters> {
    let mut counters = Counters::new();
    for line in text.lines().skip(2).filter(|line| !line.trim().is_empty()) {
        let (name, data) = line.split_once(':').context("invalid network line")?;
        let words: Vec<_> = data.split_whitespace().collect();
        ensure!(words.len() >= 16, "incomplete network counters");
        counters.insert(name.trim().into(), (words[0].parse()?, words[8].parse()?));
    }
    Ok(counters)
}

fn percent(tenths: u64) -> String {
    format!("{}.{:01}%", tenths / 10, tenths % 10)
}

pub fn bytes(value: u64) -> String {
    let mut number = value as f64;
    let mut unit = "B";
    for next in ["KiB", "MiB", "GiB", "TiB"] {
        if number < 1024.0 {
            break;
        }
        number /= 1024.0;
        unit = next;
    }
    if unit == "B" {
        format!("{value}B")
    } else {
        format!("{number:.1}{unit}")
    }
}

pub fn rate(value: u64) -> String {
    format!("{}/s", bytes(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn counters_units_and_memory_availability() {
        assert_eq!(
            cpu_counters("cpu 10 2 3 40 5 6 7 8 9 10\n").unwrap(),
            (81, 45)
        );
        assert_eq!(
            memory_bytes("MemTotal: 100 kB\nMemAvailable: 40 kB\nMemFree: 1 kB").unwrap(),
            (60 * 1024, 100 * 1024)
        );
        assert!(memory_bytes("MemTotal: 1 kB\nMemAvailable: 2 kB").is_err());
        let disk = disk_counters("8 0 sda 1 0 10 0 1 0 20 0 0 0 0").unwrap();
        assert_eq!(disk["sda"], (5120, 10240));
        assert_eq!(rate(2048), "2.0KiB/s");
    }

    #[test]
    fn rates_use_elapsed_time_and_ignore_resets_and_new_devices() {
        let now = Instant::now();
        let mut baseline = RateBaseline::default();
        assert!(
            baseline
                .update(BTreeMap::from([("eth0".into(), (1000, 2000))]), now)
                .is_none()
        );
        assert_eq!(
            baseline.update(
                BTreeMap::from([
                    ("eth0".into(), (3000, 1000)),
                    ("new".into(), (99999, 99999))
                ]),
                now + Duration::from_secs(2)
            ),
            Some((1000, 0))
        );
        let mut graph = Graph::new(true);
        for i in 0..100 {
            graph.push(i, 0, "x".into(), "y".into());
        }
        assert_eq!(graph.primary.len(), HISTORY);
        assert_eq!(graph.primary.front(), Some(&40));
    }

    #[tokio::test]
    async fn sampler_filters_virtual_devices_and_recovers_after_missing_source() {
        let temp = tempfile::tempdir().unwrap();
        let proc = temp.path().join("proc");
        let sys = temp.path().join("sys");
        std::fs::create_dir_all(proc.join("net")).unwrap();
        std::fs::create_dir_all(sys.join("block/sda/device")).unwrap();
        std::fs::create_dir_all(sys.join("class/net/eth0/device")).unwrap();
        std::fs::write(proc.join("stat"), "cpu 10 0 0 90 0 0 0 0").unwrap();
        std::fs::write(
            proc.join("meminfo"),
            "MemTotal: 100 kB\nMemAvailable: 40 kB",
        )
        .unwrap();
        std::fs::write(
            proc.join("diskstats"),
            "8 0 sda 1 0 10 0 1 0 20 0 0 0 0\n8 1 sda1 1 0 10 0 1 0 20 0 0 0 0",
        )
        .unwrap();
        std::fs::write(proc.join("net/dev"), "header\nheader\neth0: 100 0 0 0 0 0 0 0 200 0 0 0 0 0 0 0\nlo: 900 0 0 0 0 0 0 0 900 0 0 0 0 0 0 0").unwrap();
        let mut sampler = Sampler::new(&proc, &sys);
        assert_eq!(sampler.read_counters(false).await.unwrap().len(), 1);
        assert_eq!(sampler.read_counters(true).await.unwrap().len(), 1);
        let first = sampler.sample().await;
        assert!(first[0].1.is_err());
        assert_eq!(first[1].1.as_ref().unwrap().value, "60.0%");
        std::fs::write(proc.join("stat"), "cpu 30 0 0 170 0 0 0 0").unwrap();
        let second = sampler.sample().await;
        assert_eq!(second[0].1.as_ref().unwrap().value, "20.0%");
        assert!(second[2].1.is_ok());
        std::fs::remove_file(proc.join("stat")).unwrap();
        assert!(sampler.sample().await[0].1.is_err());
        std::fs::write(proc.join("stat"), "cpu 40 0 0 200 0 0 0 0").unwrap();
        assert!(sampler.sample().await[0].1.is_err()); // fresh baseline, no gap spike
    }
}
