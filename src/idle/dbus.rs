//! Session bus implementation of the freedesktop idle inhibition API.

use super::IdleState;
use futures_lite::StreamExt;
use smithay_client_toolkit::reexports::calloop::{LoopHandle, channel};
use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Duration;
use zbus::connection::Builder;
use zbus::fdo::{DBusProxy, Error, RequestNameFlags, RequestNameReply};
use zbus::message::{Flags, Header, Message, Type};
use zbus::names::OwnedUniqueName;
use zbus::{Connection, MessageStream};

const SERVICE: &str = "org.freedesktop.ScreenSaver";
const PATH: &str = "/org/freedesktop/ScreenSaver";
const LEGACY_PATH: &str = "/ScreenSaver";
const BUS: &str = "org.freedesktop.DBus";
const MAX_INHIBITORS: usize = 256;
const TIMEOUT: Duration = Duration::from_secs(3);
const INTROSPECTION: &str = r#"<node>
  <interface name="org.freedesktop.ScreenSaver">
    <method name="Inhibit">
      <arg name="application_name" type="s" direction="in"/>
      <arg name="reason_for_inhibit" type="s" direction="in"/>
      <arg name="cookie" type="u" direction="out"/>
    </method>
    <method name="UnInhibit">
      <arg name="cookie" type="u" direction="in"/>
    </method>
  </interface>
  <interface name="org.freedesktop.DBus.Introspectable">
    <method name="Introspect"><arg name="xml" type="s" direction="out"/></method>
  </interface>
  <interface name="org.freedesktop.DBus.Peer"><method name="Ping"/></interface>
</node>"#;

struct Update {
    inhibited: bool,
    applied: mpsc::Sender<()>,
}

pub fn listen(handle: &LoopHandle<'_, IdleState>) -> Result<(), String> {
    let (tx, rx) = channel::channel::<Update>();
    handle
        .insert_source(rx, |event, _, state| match event {
            channel::Event::Msg(update) => {
                state.set_dbus_inhibited(update.inhibited);
                let _ = update.applied.send(());
            }
            channel::Event::Closed => state.set_dbus_inhibited(false),
        })
        .map_err(|error| format!("D-Bus event source: {error}"))?;
    std::thread::Builder::new()
        .name("amorphis-dbus".into())
        .spawn(move || {
            if let Err(error) = zbus::block_on(serve(tx)) {
                eprintln!("amorphis-idle: D-Bus inhibition unavailable: {error}");
            }
        })
        .map_err(|error| format!("D-Bus worker: {error}"))?;
    Ok(())
}

async fn serve(tx: channel::Sender<Update>) -> Result<(), Box<dyn std::error::Error>> {
    let connection = Builder::session().method_timeout(TIMEOUT).build().await?;
    let bus = DBusProxy::new(&connection).await?;
    // Consume method calls and owner changes in the order they arrive over
    // the same connection. An exiting caller cannot leave a queued request
    // behind after its disconnect has already been processed.
    let mut messages = MessageStream::from(&connection);
    bus.add_match_rule(
        zbus::MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(BUS)
            .path("/org/freedesktop/DBus")
            .interface(BUS)
            .member("NameOwnerChanged")
            .build()?,
    )
    .await?;
    if bus
        .request_name(SERVICE.try_into()?, RequestNameFlags::DO_NOT_QUEUE)
        .await?
        != RequestNameReply::PrimaryOwner
    {
        return Err(format!("{SERVICE} already belongs to another service").into());
    }
    let mut service = ScreenSaver {
        leases: HashMap::new(),
        last_cookie: 0,
        tx,
        event_loop_failed: false,
    };
    while let Some(message) = messages.next().await {
        let message = message?;
        let header = message.header();
        match message.message_type() {
            Type::MethodCall => {
                service.dispatch(&connection, &message, &header).await?;
                // If an update timed out, close the connection and let
                // the event loop clear all bus inhibitors when it can.
                if service.event_loop_failed {
                    return Err("idle event loop could not apply a D-Bus update".into());
                }
            }
            Type::Signal
                if header.sender().is_some_and(|sender| sender.as_str() == BUS)
                    && header
                        .interface()
                        .is_some_and(|interface| interface.as_str() == BUS)
                    && header
                        .member()
                        .is_some_and(|member| member.as_str() == "NameOwnerChanged") =>
            {
                let body = message.body();
                let (name, _, new_owner): (&str, &str, &str) = body.deserialize()?;
                if name.starts_with(':') && new_owner.is_empty() {
                    service.remove_owner(name)?;
                }
            }
            _ => {}
        }
    }
    Err("session bus disconnected".into())
}

struct ScreenSaver {
    leases: HashMap<u32, OwnedUniqueName>,
    last_cookie: u32,
    tx: channel::Sender<Update>,
    event_loop_failed: bool,
}

impl ScreenSaver {
    async fn dispatch(
        &mut self,
        connection: &Connection,
        message: &Message,
        header: &Header<'_>,
    ) -> zbus::Result<()> {
        let result = self.call(message, header);
        if header.primary().flags().contains(Flags::NO_REPLY_EXPECTED) {
            return Ok(());
        }
        match result {
            Ok(Reply::Cookie(cookie)) => connection.reply(header, &cookie).await,
            Ok(Reply::Xml) => connection.reply(header, &INTROSPECTION).await,
            Ok(Reply::Empty) => connection.reply(header, &()).await,
            Err(error) => connection.reply_dbus_error(header, error).await,
        }
    }

    fn call(&mut self, message: &Message, header: &Header<'_>) -> Result<Reply, Error> {
        if !matches!(
            header.path().map(|path| path.as_str()),
            Some(PATH | LEGACY_PATH)
        ) {
            return Err(Error::UnknownObject("unknown screensaver object".into()));
        }
        let interface = header.interface().map(|value| value.as_str());
        let method = header.member().map(|value| value.as_str());
        let body = message.body();
        match (interface, method) {
            (Some(SERVICE) | None, Some("Inhibit")) => {
                let _: (&str, &str) = body.deserialize().map_err(invalid_args)?;
                let sender = header.sender().ok_or_else(no_sender)?;
                self.acquire(sender.to_owned().into()).map(Reply::Cookie)
            }
            (Some(SERVICE) | None, Some("UnInhibit")) => {
                let cookie: u32 = body.deserialize().map_err(invalid_args)?;
                let sender = header.sender().ok_or_else(no_sender)?;
                self.release(cookie, sender.as_str())?;
                Ok(Reply::Empty)
            }
            (Some("org.freedesktop.DBus.Introspectable") | None, Some("Introspect")) => {
                Ok(Reply::Xml)
            }
            (Some("org.freedesktop.DBus.Peer") | None, Some("Ping")) => Ok(Reply::Empty),
            _ => Err(Error::UnknownMethod("unknown screensaver method".into())),
        }
    }

    fn acquire(&mut self, sender: OwnedUniqueName) -> Result<u32, Error> {
        if self.leases.len() >= MAX_INHIBITORS {
            return Err(Error::LimitsExceeded("too many idle inhibitors".into()));
        }
        loop {
            self.last_cookie = self.last_cookie.wrapping_add(1);
            if self.last_cookie != 0 && !self.leases.contains_key(&self.last_cookie) {
                break;
            }
        }
        if self.leases.is_empty() {
            self.update(true)?;
        }
        self.leases.insert(self.last_cookie, sender);
        Ok(self.last_cookie)
    }

    fn release(&mut self, cookie: u32, sender: &str) -> Result<(), Error> {
        match self.leases.get(&cookie) {
            Some(owner) if owner.as_str() == sender => {}
            Some(_) => {
                return Err(Error::AccessDenied(
                    "cookie belongs to another connection".into(),
                ));
            }
            None => return Err(Error::InvalidArgs("unknown idle inhibitor cookie".into())),
        }
        self.leases.remove(&cookie);
        if self.leases.is_empty() {
            self.update(false)?;
        }
        Ok(())
    }

    fn remove_owner(&mut self, sender: &str) -> Result<(), Error> {
        let had_leases = !self.leases.is_empty();
        self.leases.retain(|_, owner| owner.as_str() != sender);
        if had_leases && self.leases.is_empty() {
            self.update(false)?;
        }
        Ok(())
    }

    fn update(&mut self, inhibited: bool) -> Result<(), Error> {
        let (applied, reply) = mpsc::channel();
        self.tx
            .send(Update { inhibited, applied })
            .map_err(|_| Error::Failed("idle event loop stopped".into()))?;
        if reply.recv_timeout(TIMEOUT).is_err() {
            self.event_loop_failed = true;
            return Err(Error::Failed(
                "idle event loop did not apply the change".into(),
            ));
        }
        Ok(())
    }
}

enum Reply {
    Cookie(u32),
    Xml,
    Empty,
}

fn invalid_args(error: zbus::Error) -> Error {
    Error::InvalidArgs(error.to_string())
}

fn no_sender() -> Error {
    Error::AccessDenied("D-Bus sender is required".into())
}
