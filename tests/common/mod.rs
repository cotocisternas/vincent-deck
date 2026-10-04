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
 { factory = adapter args = { factory.name = support.null-audio-sink node.name = test-output media.class = Audio/Sink audio.position = [ FL FR ] } }
 { factory = adapter args = { factory.name = support.null-audio-sink node.name = test-input media.class = Audio/Source audio.position = [ MONO ] } }
 { factory = adapter args = { factory.name = support.null-audio-sink node.name = alternate-output media.class = Audio/Sink audio.position = [ FL FR ] } }
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
        }
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

    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for PipeWire {
    fn drop(&mut self) {
        self.stop();
    }
}
