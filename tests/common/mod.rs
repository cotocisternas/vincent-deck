use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

/// A real, private PipeWire daemon with software devices, never the desktop's
/// audio session. CLI tools below are independent test observers/controllers.
pub struct PipeWire {
    pub root: PathBuf,
    pub child: Child,
    policy: Option<Child>,
}

impl PipeWire {
    pub fn start(root: &Path) -> Self {
        let config = root.join("pipewire-test.conf");
        std::fs::write(&config, r#"
context.properties = { core.daemon = true core.name = vincent-test }
context.spa-libs = { support.* = support/libspa-support audio.convert.* = audioconvert/libspa-audioconvert }
context.modules = [
 { name = libpipewire-module-protocol-native }
 { name = libpipewire-module-metadata }
 { name = libpipewire-module-spa-node-factory }
 { name = libpipewire-module-adapter }
 { name = libpipewire-module-access args = { access.socket = { vincent-test = "unrestricted" } } }
]
context.objects = [
 { factory = adapter args = { factory.name = support.null-audio-sink node.name = test-output node.nick = "Test Speakers" media.class = Audio/Sink audio.position = [ FL FR ] } }
 { factory = adapter args = { factory.name = support.null-audio-sink node.name = test-input node.nick = "Test Mic" media.class = Audio/Source audio.position = [ MONO ] } }
 { factory = adapter args = { factory.name = support.null-audio-sink node.name = alternate-output node.nick = "Headphones" media.class = Audio/Sink audio.position = [ FL FR ] } }
 { factory = adapter args = { factory.name = support.null-audio-sink node.name = alternate-input node.nick = "Headset Mic" media.class = Audio/Source audio.position = [ MONO ] } }
 { factory = metadata args = { metadata.name = default metadata.values = [
   { key = default.audio.sink type = "Spa:String:JSON" value = { name = test-output } }
   { key = default.audio.source type = "Spa:String:JSON" value = { name = test-input } }
 ] } }
]
"#).unwrap();
        let child = Command::new("pipewire")
            .args(["-c", config.to_str().unwrap()])
            .env("PIPEWIRE_RUNTIME_DIR", root)
            .env("XDG_RUNTIME_DIR", root)
            .stdout(Stdio::null())
            .spawn()
            .expect("pipewire is required for native audio integration tests");
        let deadline = Instant::now() + Duration::from_secs(5);
        while !root.join("vincent-test").exists() {
            assert!(
                Instant::now() < deadline,
                "private PipeWire socket did not appear"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        Self {
            root: root.to_owned(),
            child,
            policy: None,
        }
    }

    /// Minimal session-manager policy for the private daemon: apply configured
    /// defaults to effective defaults. The desktop runs real WirePlumber policy.
    pub fn start_policy(&mut self) {
        let script = r#"
import subprocess,re
watch=subprocess.Popen(['pw-metadata','-n','default','-m'],stdout=subprocess.PIPE,text=True)
try:
 for line in watch.stdout:
  match=re.search(r"key:'(default.configured.audio.(?:sink|source))' value:'([^']+)'",line)
  if match:
   key,value=match.groups()
   subprocess.run(['pw-metadata','-n','default','0',key.replace('.configured',''),value,'Spa:String:JSON'],stdout=subprocess.DEVNULL,check=True)
finally:
 watch.terminate()
 watch.wait()
"#;
        self.policy = Some(
            self.command("python3")
                .args(["-u", "-c", script])
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        );
    }

    pub fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command
            .env("PIPEWIRE_RUNTIME_DIR", &self.root)
            .env("XDG_RUNTIME_DIR", &self.root)
            .env("PIPEWIRE_REMOTE", "vincent-test");
        command
    }

    pub fn control(&self, args: &[&str]) -> String {
        let result = self.command("wpctl").args(args).output().unwrap();
        assert!(
            result.status.success(),
            "wpctl: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stdout).unwrap()
    }

    pub fn default_name(&self, key: &str) -> String {
        let output = self
            .command("pw-metadata")
            .args(["-n", "default"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        let line = text
            .lines()
            .find(|line| line.contains(&format!("key:'{key}'")))
            .unwrap();
        let value = line
            .split("value:'")
            .nth(1)
            .unwrap()
            .split('\'')
            .next()
            .unwrap();
        serde_json::from_str::<serde_json::Value>(value).unwrap()["name"]
            .as_str()
            .unwrap()
            .into()
    }

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(mut policy) = self.policy.take() {
            let _ = policy.wait();
        }
    }
}

impl Drop for PipeWire {
    fn drop(&mut self) {
        self.stop();
    }
}
