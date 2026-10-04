use crate::{app::App, render::Action as Kind};
use openaction::{Action, Instance, OpenActionResult};
use std::sync::Arc;

macro_rules! action {
    ($type:ident, $kind:ident, $name:literal) => {
        pub struct $type(pub Arc<App>);
        #[openaction::async_trait]
        impl Action for $type {
            const UUID: &'static str = concat!("dev.vincent.deck.", $name);
            type Settings = serde_json::Value;
            async fn will_appear(
                &self,
                instance: &Instance,
                _: &Self::Settings,
            ) -> OpenActionResult<()> {
                self.0.appear(Kind::$kind, instance).await;
                Ok(())
            }
            async fn will_disappear(
                &self,
                instance: &Instance,
                _: &Self::Settings,
            ) -> OpenActionResult<()> {
                self.0.disappear(&instance.instance_id).await;
                Ok(())
            }
            async fn key_down(
                &self,
                instance: &Instance,
                _: &Self::Settings,
            ) -> OpenActionResult<()> {
                if !Kind::$kind.panel() && instance.controller == "Keypad" {
                    self.0
                        .input(Kind::$kind, instance.instance_id.clone(), None)
                        .await;
                }
                Ok(())
            }
            async fn dial_down(
                &self,
                instance: &Instance,
                _: &Self::Settings,
            ) -> OpenActionResult<()> {
                if Kind::$kind.panel() && instance.controller == "Encoder" {
                    self.0.dial_click(Kind::$kind, instance).await;
                }
                Ok(())
            }
            async fn touch_tap(
                &self,
                instance: &Instance,
                _: &Self::Settings,
                _: (u16, u16),
                _: bool,
            ) -> OpenActionResult<()> {
                if Kind::$kind.panel() && instance.controller == "Encoder" {
                    self.0
                        .input(Kind::$kind, instance.instance_id.clone(), None)
                        .await;
                }
                Ok(())
            }
            async fn dial_rotate(
                &self,
                instance: &Instance,
                _: &Self::Settings,
                ticks: i16,
                _: bool,
            ) -> OpenActionResult<()> {
                if Kind::$kind.panel() && instance.controller == "Encoder" {
                    self.0
                        .input(Kind::$kind, instance.instance_id.clone(), Some(ticks))
                        .await;
                }
                Ok(())
            }
        }
    };
}
action!(Terminal, Terminal, "terminal");
action!(Browser, Browser, "browser");
action!(Screenshot, Screenshot, "screenshot");
action!(Record, Record, "record");
action!(Agent, Agent, "agent");
action!(Clipboard, Clipboard, "clipboard");
action!(Night, Night, "night");
action!(Lock, Lock, "lock");
action!(Volume, Volume, "volume");
action!(Mic, Mic, "mic");
action!(Workspace, Workspace, "workspace");
action!(Theme, Theme, "theme");
action!(Cpu, Cpu, "cpu");
action!(Memory, Memory, "memory");
action!(Disk, Disk, "disk");
action!(Network, Network, "network");

pub async fn register(app: Arc<App>) {
    openaction::global_events::set_global_event_handler(Box::leak(Box::new(Global(app.clone()))));
    macro_rules! register { ($($type:ident),*) => { $(openaction::register_action($type(app.clone())).await;)* }; }
    register!(
        Terminal, Browser, Screenshot, Record, Agent, Clipboard, Night, Lock, Volume, Mic,
        Workspace, Theme, Cpu, Memory, Disk, Network
    );
}

struct Global(Arc<App>);
#[openaction::async_trait]
impl openaction::global_events::GlobalEventHandler for Global {
    async fn plugin_ready(&self) -> OpenActionResult<()> {
        self.0.reconnect().await;
        Ok(())
    }
    async fn device_did_connect(
        &self,
        _: openaction::global_events::DeviceDidConnectEvent,
    ) -> OpenActionResult<()> {
        self.0.reconnect().await;
        Ok(())
    }
    async fn device_did_disconnect(
        &self,
        event: openaction::global_events::DeviceDidDisconnectEvent,
    ) -> OpenActionResult<()> {
        self.0.disconnect_device(&event.device).await;
        Ok(())
    }
}
